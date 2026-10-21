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
use super::property::{FieldValue, GetMeta, Getter, Property, PropertyTable, SetMeta, Setter, Slot, get_direct, set_direct};
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

	/// Add a read-only property `name` backed by a typed getter over the payload: it has
	/// no setter (no slot in `__setTable`), so writing it raises "the index '…' does not
	/// exist", as a Sqrat property without one does.
	pub fn property_ro<G: FieldValue + 'static>(mut self, name: &str, get: impl Fn(&T) -> G + 'static) -> Self {
		self.properties.push((name.to_string(), Property::new_ro(get)));
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

	/// Add a read/write **object-valued** property `name`: `get` returns the value
	/// to read (`None` reads `null`); `set` receives whatever is assigned, as an owned
	/// [`SqStrongRef`], and may reject it with an error. For array- or
	/// instance-valued fields a scalar [`property`](Self::property) can't express.
	pub fn property_obj_rw(
		mut self,
		name: &str,
		get: impl Fn(&T, &SQVM) -> Option<SqStrongRef> + 'static,
		set: impl Fn(&mut T, &SQVM, SqStrongRef) -> Result<(), String> + 'static,
	) -> Self {
		self.properties
			.push((name.to_string(), Property::new_obj_rw(get, set)));
		self
	}

	/// Add a write-ignored, read-null property `name` (assignment discards any value,
	/// reads yield `null`). For object-valued fields a headless stub doesn't model —
	/// e.g. a render `Color` scripts assign but nothing logic-relevant reads back.
	pub fn property_noop(mut self, name: &str) -> Self {
		self.properties.push((name.to_string(), Property::new_noop()));
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
		// Sqrat's layout (see `property`): a native accessor closure per property, in the
		// class's `__getTable` and (if it has a setter) `__setTable`.
		let mut slots = Vec::new();
		for (name, prop) in self.properties {
			let prop = Rc::new(prop);
			// A scalar property's accessors the VM may also call directly (`get_direct`).
			let direct = prop.direct.then_some(Rc::as_ptr(&prop) as squirrel_sys::SQUserPointer);
			let getter = accessor_ref(vm, Box::new(Getter { prop: prop.clone() }), direct.map(|ud| (get_direct::<T> as Direct, ud)));
			let setter = prop
				.writable
				.then(|| accessor_ref(vm, Box::new(Setter { prop: prop.clone() }), direct.map(|ud| (set_direct::<T> as Direct, ud))));
			slots.push((name, Slot::new(prop, getter, setter)));
		}
		let (get_table, set_table) = {
			let mut get_table = SqTable::new(vm);
			let mut set_table = SqTable::new(vm);
			for (name, slot) in &slots {
				get_table.set(name, slot.getter.clone());
				if let Some(setter) = &slot.setter {
					set_table.set(name, setter.clone());
				}
			}
			(get_table.to_strong(), set_table.to_strong())
		};
		let table: PropertyTable<T> = Rc::new(
			slots
				.into_iter()
				.map(|(name, slot)| (name.into_bytes().into_boxed_slice(), slot))
				.collect::<HashMap<_, _, _>>(),
		);

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
			table: get_table.clone(),
		})); // [class, "_get", closure]
		// The VM looks accessors up in `__getTable` itself, as `GetMeta` does.
		get_table.push(vm); // [class, "_get", closure, table]
		unsafe { squirrel_sys::sq_setaccessortable(ptr, -2) }; // [class, "_get", closure]
		unsafe { squirrel_sys::sq_newslot(ptr, -3, 0) }; // [class]

		vm.push_string("_set"); // [class, "_set"]
		vm.push_closure(Box::new(SetMeta {
			props: table,
			table: set_table.clone(),
		})); // [class, "_set", closure]
		set_table.push(vm); // [class, "_set", closure, table]
		unsafe { squirrel_sys::sq_setaccessortable(ptr, -2) }; // [class, "_set", closure]
		unsafe { squirrel_sys::sq_newslot(ptr, -3, 0) }; // [class]

		for (name, accessors) in [("__getTable", get_table), ("__setTable", set_table)] {
			vm.push_string(name); // [class, name]
			accessors.push(vm); // [class, name, table]
			unsafe { squirrel_sys::sq_newslot(ptr, -3, 0) }; // [class]
		}

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
	/// Run `f` on the payload of `instance`, if it is a native instance carrying a `T`
	/// (checked by typetag and `TypeId`); `None` otherwise. For host code that keeps
	/// strong refs to its instances and needs their state back (e.g. to save it).
	/// No script runs inside `f`, so the borrow can't be observed by script code.
	///
	/// The payload's place is read from the VM once per ref (and its clones) and kept
	/// in it: an instance's payload never moves, and the ref keeps the instance alive.
	pub fn with_payload<R>(&self, vm: &SQVM, instance: &SqStrongRef, f: impl FnOnce(&mut T) -> R) -> Option<R> {
		instance.with_native(vm, f)
	}

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

/// A property accessor's direct function for the VM and its property.
type Direct = unsafe extern "C" fn(
	squirrel_sys::HSQUIRRELVM,
	squirrel_sys::SQUserPointer,
	squirrel_sys::SQUserPointer,
	squirrel_sys::SQUserPointer,
) -> squirrel_sys::SQInteger;

/// A native accessor closure for `func`, as an owned reference, which the VM may call as
/// `direct` instead (`sq_setnativeaccessor`). The closure's `func` holds the property
/// `direct` points at, so it lives as long as the closure.
fn accessor_ref(vm: &mut SQVM, func: Box<dyn SqFunction>, direct: Option<(Direct, squirrel_sys::SQUserPointer)>) -> SqStrongRef {
	vm.push_closure(func);
	if let Some((fun, ud)) = direct {
		unsafe { squirrel_sys::sq_setnativeaccessor(vm.as_ptr(), -1, Some(fun), ud) };
	}
	let closure = SqStrongRef::from_stack(vm, -1);
	unsafe { squirrel_sys::sq_pop(vm.as_ptr(), 1) };
	closure
}

impl SqStrongRef {
	/// Run `f` on the payload of the native instance this refers to, if it carries a `T`
	/// (checked by typetag and `TypeId`); `None` otherwise (another type, or not a native
	/// instance). As [`SqClass::with_payload`], without needing the class: for a native
	/// that reads a native argument (a vector, a tile) but also takes other values.
	pub fn with_native<T: 'static, R>(&self, vm: &SQVM, f: impl FnOnce(&mut T) -> R) -> Option<R> {
		let mut up = self.cached_payload();
		if up.is_null() {
			self.push(vm); // [instance]
			up = unsafe { super::payload::any_payload(vm, -1) }.unwrap_or(std::ptr::null_mut());
			unsafe { squirrel_sys::sq_pop(vm.as_ptr(), 1) }; // []
			if up.is_null() {
				return None;
			}
			self.cache_payload(up);
		}
		// This strong ref keeps the instance (and so the payload) alive.
		let payload = unsafe { super::payload::payload_of::<T>(up) }?;
		Some(f(unsafe { &mut (*payload).value }))
	}
}
