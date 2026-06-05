//! Defining a native class and creating instances of it.
//!
//! [`ClassBuilder`] accumulates a class's properties, then [`build`](ClassBuilder::build)
//! registers a Squirrel class (in the root table under its name) backed by the
//! `_get`/`_set` metamethods and returns an [`SqClass`] handle. [`SqClass::instantiate`]
//! creates an instance carrying a fresh Rust payload.

use std::collections::HashMap;
use std::marker::PhantomData;
use std::rc::Rc;

use super::method::IntoSqMethod;
use super::payload::{NativePayload, instance_release_hook, native_payload_tag};
use super::property::{FieldValue, GetMeta, Property, PropertyTable, SetMeta};
use crate::function::{IntoSqFunction, SqFunction};
use crate::{SQ_OK, SQVM, SqStrongRef, SqTable};

/// Builds a native class whose instances carry a Rust payload of type `T`.
///
/// Add properties with [`property`](Self::property), then [`build`](Self::build):
///
/// ```ignore
/// #[derive(Clone)]
/// struct Vec2f { x: f64, y: f64 }
///
/// let vec2f = ClassBuilder::<Vec2f>::new("vec2f")
///     .property("X", |v| v.x, |v, x| v.x = x)
///     .property("Y", |v| v.y, |v, y| v.y = y)
///     .method("scale", |v, vm| { let f = vm.get_float(2).unwrap_or(1.0); v.x *= f; v.y *= f; 0 })
///     .build(&mut vm);
///
/// let instance = vec2f.instantiate(&vm, Vec2f { x: 1.0, y: 2.0 });
/// ```
pub struct ClassBuilder<T> {
	name: String,
	properties: Vec<(String, Property<T>)>,
	methods: Vec<(String, Box<dyn SqFunction>)>,
}

impl<T: 'static> ClassBuilder<T> {
	/// Start a class named `name` (the slot it will occupy in the root table).
	pub fn new(name: &str) -> Self {
		ClassBuilder {
			name: name.to_string(),
			properties: Vec::new(),
			methods: Vec::new(),
		}
	}

	/// Add a property `name` backed by a typed getter and setter over the payload.
	/// `G` is the field's scalar type (`f64` / `i64` / `bool`).
	pub fn property<G: FieldValue + 'static>(
		mut self,
		name: &str,
		get: impl Fn(&T) -> G + 'static,
		set: impl Fn(&mut T, G) + 'static,
	) -> Self {
		self.properties
			.push((name.to_string(), Property::new(get, set)));
		self
	}

	/// Add a read-only **object-valued** property `name`: the getter returns an
	/// owned [`SqStrongRef`](crate::SqStrongRef) (e.g. a `vec2i` instance or a plain
	/// `{X,Y}` table) which is pushed for `inst.name`. Use this for field-style reads
	/// that can't be a scalar [`property`](Self::property) — e.g. `tile.Coords`.
	/// Writing the property raises an error.
	pub fn property_obj(
		mut self,
		name: &str,
		get: impl Fn(&T, &SQVM) -> SqStrongRef + 'static,
	) -> Self {
		self.properties
			.push((name.to_string(), Property::new_obj_ro(get)));
		self
	}

	/// Add a callable method `name` to the class.
	///
	/// The body is a typed closure `FnMut(&mut T, args..) -> R`: the receiver
	/// `&mut T` is the instance's payload, each argument is read via its [`SqArg`]
	/// (so the signature drives the param-check, just like
	/// [`register`](SQVM::register)), and the return goes back via its
	/// [`IntoSqReturn`](crate) — the body never touches the stack. Because the
	/// method is a real class member, `inst.name(...)` resolves by ordinary lookup
	/// and never hits the `_get` fallback.
	///
	/// ```ignore
	/// .method("setX", |v, x: f64| v.x = x)            // mutator, returns ()
	/// .method("sum", |v| v.x + v.y)                   // returns a value
	/// ```
	pub fn method<Marker, F>(mut self, name: &str, body: F) -> Self
	where
		F: IntoSqMethod<T, Marker>,
	{
		self.methods
			.push((name.to_string(), body.into_method_fn(name.to_string())));
		self
	}

	/// Add a callable member `name` backed by a *raw* native closure — the
	/// table-style [`IntoSqFunction`] shape (e.g. `FnMut(&SQVM, This, Variadic<_>)`)
	/// rather than the typed `&mut T` receiver shape of [`method`](Self::method).
	///
	/// Use this for a member that must see the raw receiver instead of a typed
	/// payload — chiefly a Sqrat-style overload **dispatcher**, which re-resolves and
	/// calls one of the class's `__sqrat_ol_ name_N` leaves on `this`. Like `method`,
	/// it is slotted as a real class member, so `inst.name(...)` resolves directly.
	pub fn native_method<Marker, F>(mut self, name: &str, body: F) -> Self
	where
		F: IntoSqFunction<Marker>,
	{
		self.methods
			.push((name.to_string(), body.into_sq_function()));
		self
	}

	/// Register the class in the root table and return a handle for instantiation.
	pub fn build(self, vm: &mut SQVM) -> SqClass<T> {
		let ptr = vm.as_ptr();
		let table: PropertyTable<T> =
			Rc::new(self.properties.into_iter().collect::<HashMap<_, _>>());

		unsafe {
			squirrel_sys::sq_newclass(ptr, 0); // [class] (no base class)
			// Tag every native class with the shared typetag so its instances are
			// recognizable as payload-carrying.
			squirrel_sys::sq_settypetag(ptr, -1, native_payload_tag());
		}

		// Slot the `_get` / `_set` metamethods onto the class.
		vm.push_string("_get"); // [class, "_get"]
		vm.push_closure(Box::new(GetMeta {
			props: table.clone(),
		})); // [class, "_get", closure]
		unsafe { squirrel_sys::sq_newslot(ptr, -3, 0) }; // [class]

		vm.push_string("_set"); // [class, "_set"]
		vm.push_closure(Box::new(SetMeta { props: table })); // [class, "_set", closure]
		unsafe { squirrel_sys::sq_newslot(ptr, -3, 0) }; // [class]

		// Slot each method as a real class member, so `inst.method(...)` resolves
		// by normal lookup ahead of the `_get` fallback.
		for (name, func) in self.methods {
			vm.push_string(&name); // [class, name]
			vm.push_closure(func); // [class, name, closure]
			unsafe { squirrel_sys::sq_newslot(ptr, -3, 0) }; // [class]
		}

		// Two strong refs to the class object on the stack: one to keep in the
		// handle, one to hand to the root table (no `Clone` needed).
		let for_handle = SqStrongRef::from_stack(vm, -1);
		let for_root = SqStrongRef::from_stack(vm, -1);
		unsafe { squirrel_sys::sq_pop(ptr, 1) }; // []

		SqTable::root(vm).set(&self.name, for_root);

		SqClass {
			class: for_handle,
			_marker: PhantomData,
		}
	}
}

/// A handle to a registered native class, used to create instances.
pub struct SqClass<T> {
	class: SqStrongRef,
	_marker: PhantomData<fn() -> T>,
}

/// Cloning shares the same underlying class (a fresh addref), so the same class
/// can back several factory closures. Independent of whether `T: Clone`.
impl<T> Clone for SqClass<T> {
	fn clone(&self) -> Self {
		SqClass {
			class: self.class.clone(),
			_marker: PhantomData,
		}
	}
}

impl<T: 'static> SqClass<T> {
	/// Create an instance carrying `value` as its payload, returned as an owned
	/// strong reference (so it can be the return value of a factory native fn).
	///
	/// `sq_createinstance` cannot fail for a class built by [`ClassBuilder::build`],
	/// but the failure path is handled defensively (no leak, balanced stack): a
	/// null reference is returned and `value` is dropped normally.
	pub fn instantiate(&self, vm: &SQVM, value: T) -> SqStrongRef {
		let ptr = vm.as_ptr();
		self.class.push(vm); // [class]
		unsafe {
			if squirrel_sys::sq_createinstance(ptr, -1) != SQ_OK {
				// Nothing was pushed; drop the class and hand back a null ref. The
				// `value` is moved out below, so it just drops here.
				squirrel_sys::sq_pop(ptr, 1); // []
				squirrel_sys::sq_pushnull(ptr);
				let null = SqStrongRef::from_stack(vm, -1);
				squirrel_sys::sq_pop(ptr, 1);
				return null;
			}
			// [class, instance]. The payload box is created only now, so a failed
			// createinstance never leaks it.
			let payload = NativePayload::into_raw(value);
			squirrel_sys::sq_setinstanceup(ptr, -1, payload);
			squirrel_sys::sq_setreleasehook(ptr, -1, Some(instance_release_hook::<T>));
			let instance = SqStrongRef::from_stack(vm, -1);
			squirrel_sys::sq_pop(ptr, 2); // pop instance + class
			instance
		}
	}
}
