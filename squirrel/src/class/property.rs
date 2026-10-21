//! Native properties and the `_get` / `_set` metamethods that back them.
//!
//! A native class declares no real Squirrel members; instead every field access
//! `inst.X` (and `inst.X = v`) misses and routes to the class's `_get` / `_set`
//! metamethods. Those read the instance's Rust payload and dispatch on the field
//! name to a registered getter/setter — so `R`/`G`/`B`/`A`/`RGBA` on a `color`
//! can all be views over one `u32`, staying consistent.
//!
//! The layout is Sqrat's, which the original engine's bindings use and mods rely on:
//! the class holds a `__getTable` and a `__setTable` of native accessor closures, one
//! per property (a getter is called with the instance as `this`, a setter with the
//! instance and the value), and `_get`/`_set` look the accessor up there on every
//! access. So a mod can wrap a property (Reforged replaces
//! `::TacticalTile.__getTable.IsEmpty`) or add one. While a slot still holds the
//! class's own accessor, the metamethod calls the Rust property directly.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::rc::Rc;

use super::payload::payload_ptr;
use crate::function::SqFunction;
use crate::{SQVM, SqStrongRef};

/// A scalar value a native field can read from / write to the Squirrel stack.
///
/// Kept separate from the container-oriented `SqValue` so a field getter/setter
/// can be `Fn(&T) -> G` / `Fn(&mut T, G)` for an owned `G` without GAT gymnastics.
/// Defined `pub` (so it can bound the public `property` method) but not exported.
pub trait FieldValue: Sized {
	fn push_field(self, vm: &SQVM);
	fn read_field(vm: &SQVM, idx: i64) -> Option<Self>;
}

impl FieldValue for f64 {
	fn push_field(self, vm: &SQVM) {
		vm.push_float(self);
	}
	// A Sqrat property setter reads its value through the same `Var<float>` as a
	// native's argument (e.g. the sprite `Scale` setter thunk at `0x4c1e90`), so a
	// non-number is written as `0.0` instead of raising.
	fn read_field(vm: &SQVM, idx: i64) -> Option<f64> {
		Some(vm.get_float_lenient(idx))
	}
}

impl FieldValue for i64 {
	fn push_field(self, vm: &SQVM) {
		vm.push_integer(self);
	}
	// As for `f64`: a float truncates and a non-number is written as `0`.
	fn read_field(vm: &SQVM, idx: i64) -> Option<i64> {
		Some(vm.get_int_lenient(idx))
	}
}

impl FieldValue for bool {
	fn push_field(self, vm: &SQVM) {
		vm.push_bool(self);
	}
	// A Sqrat property setter reads its value through `Var<bool>`, which is `sq_tobool`
	// (`0x586df0`): any value, by its truth (verified in the original: `UseHardwareCursor = 0`
	// reads back false).
	fn read_field(vm: &SQVM, idx: i64) -> Option<bool> {
		let mut b: squirrel_sys::SQBool = 0;
		unsafe { squirrel_sys::sq_tobool(vm.as_ptr(), idx, &mut b) };
		Some(b != 0)
	}
}

impl FieldValue for String {
	fn push_field(self, vm: &SQVM) {
		vm.push_string(&self);
	}
	fn read_field(vm: &SQVM, idx: i64) -> Option<String> {
		vm.get_string(idx).ok()
	}
}

impl FieldValue for crate::SqBytes {
	fn push_field(self, vm: &SQVM) {
		vm.push_bytes(&self.0);
	}
	fn read_field(vm: &SQVM, idx: i64) -> Option<crate::SqBytes> {
		vm.get_bytes_ref(idx).map(|b| crate::SqBytes(b.to_vec()))
	}
}

/// A single named property: a getter that pushes the field and a setter that
/// reads a value off the stack and writes the field. Both are type-erased over
/// the field's scalar type.
pub(crate) struct Property<T> {
	#[allow(clippy::type_complexity)]
	pub(crate) get: Box<dyn Fn(&T, &SQVM)>,
	#[allow(clippy::type_complexity)]
	pub(crate) set: Box<dyn Fn(&mut T, &SQVM, i64) -> Result<(), String>>,
	/// Whether it has a setter (a slot in `__setTable`).
	pub(crate) writable: bool,
	/// Whether the VM may call its accessors without a call frame ([`get_direct`],
	/// [`set_direct`]): a scalar property, whose accessors never run script code.
	pub(crate) direct: bool,
}

impl<T: 'static> Property<T> {
	/// Build a property from a typed getter/setter over field type `G`.
	pub(crate) fn new<G: FieldValue + 'static>(
		get: impl Fn(&T) -> G + 'static,
		set: impl Fn(&mut T, G) + 'static,
	) -> Self {
		Property {
			get: Box::new(move |t, vm| get(t).push_field(vm)),
			set: Box::new(move |t, vm, idx| match G::read_field(vm, idx) {
				Some(value) => {
					set(t, value);
					Ok(())
				}
				None => Err("value has the wrong type for this property".to_string()),
			}),
			writable: true,
			direct: true,
		}
	}

	/// A read-only property of field type `G`: only a getter, in `__getTable`, so
	/// writing it raises "the index '…' does not exist", as Sqrat's do.
	pub(crate) fn new_ro<G: FieldValue + 'static>(get: impl Fn(&T) -> G + 'static) -> Self {
		Property {
			get: Box::new(move |t, vm| get(t).push_field(vm)),
			set: Box::new(|_t, _vm, _idx| Err("this property is read-only".to_string())),
			writable: false,
			direct: true,
		}
	}

	/// A read-only **object** property: the getter builds and returns an
	/// [`SqStrongRef`] (e.g. a `vec2i` or a plain `{X,Y}` table), which `_get`
	/// pushes. Writing it raises an error. Backs field-style object reads like
	/// `tile.Coords` that a scalar [`FieldValue`] can't express.
	pub(crate) fn new_obj_ro(get: impl Fn(&T, &SQVM) -> SqStrongRef + 'static) -> Self {
		Property {
			get: Box::new(move |t, vm| get(t, vm).push(vm)),
			set: Box::new(|_t, _vm, _idx| Err("this property is read-only".to_string())),
			writable: false,
			direct: false,
		}
	}

	/// A read/write **object** property: the getter returns the value to push (`None`
	/// pushes `null`), the setter receives the assigned value (any type) as an owned
	/// [`SqStrongRef`] and may reject it. Backs fields like a navigator setting's
	/// `AlliedFactions = [..]` or `TileToConsiderEmpty = tile`.
	#[allow(clippy::type_complexity)]
	pub(crate) fn new_obj_rw(
		get: impl Fn(&T, &SQVM) -> Option<SqStrongRef> + 'static,
		set: impl Fn(&mut T, &SQVM, SqStrongRef) -> Result<(), String> + 'static,
	) -> Self {
		Property {
			get: Box::new(move |t, vm| match get(t, vm) {
				Some(value) => value.push(vm),
				None => vm.push_null(),
			}),
			set: Box::new(move |t, vm, idx| set(t, vm, SqStrongRef::from_stack(vm, idx))),
			writable: true,
			direct: false,
		}
	}

	/// A write-ignored, read-null property: assignment accepts (and discards) any value,
	/// reading yields `null`. For object-valued fields a headless stub doesn't model —
	/// e.g. a render `Color` a script sets but nothing logic-relevant reads back.
	pub(crate) fn new_noop() -> Self {
		Property {
			get: Box::new(|_t, vm| vm.push_null()),
			set: Box::new(|_t, _vm, _idx| Ok(())),
			writable: true,
			direct: true,
		}
	}
}

/// A property and the accessor closures the class put in `__getTable`/`__setTable` for
/// it, which `_get`/`_set` recognize to skip the call through Squirrel. The refs also
/// keep those closures alive, so a replaced one's identity can't be reused.
pub(crate) struct Slot<T> {
	pub(crate) prop: Rc<Property<T>>,
	pub(crate) getter: SqStrongRef,
	pub(crate) setter: Option<SqStrongRef>,
	/// The accessors' identities, compared on every access.
	pub(crate) getter_id: i64,
	pub(crate) setter_id: Option<i64>,
}

impl<T> Slot<T> {
	pub(crate) fn new(prop: Rc<Property<T>>, getter: SqStrongRef, setter: Option<SqStrongRef>) -> Self {
		let getter_id = getter.identity();
		let setter_id = setter.as_ref().map(SqStrongRef::identity);
		Slot { prop, getter, setter, getter_id, setter_id }
	}
}

/// Shared, per-class property table, captured by both metamethods. Keyed by the name's
/// bytes, so a lookup needs no UTF-8 check (a non-UTF-8 key simply matches nothing),
/// and hashed with [`NameHasher`]: every native field access hashes its name, and these
/// are short.
pub(crate) type PropertyTable<T> = Rc<HashMap<Box<[u8]>, Slot<T>, BuildHasherDefault<NameHasher>>>;

/// A property's getter as Sqrat binds it (in `__getTable`): called with the instance as
/// `this`, it reads the payload.
pub(crate) struct Getter<T> {
	pub(crate) prop: Rc<Property<T>>,
}

impl<T: 'static> SqFunction for Getter<T> {
	fn invoke(&mut self, vm: &SQVM) -> squirrel_sys::SQInteger {
		match unsafe { payload_ptr::<T>(vm, 1) } {
			Some(payload) => {
				(self.prop.get)(unsafe { &(*payload).value }, vm);
				1
			}
			None => vm.throw_error("the getter's receiver is not an instance of this native class"),
		}
	}
}

/// A property's setter as Sqrat binds it (in `__setTable`): called with the instance as
/// `this` and the value.
pub(crate) struct Setter<T> {
	pub(crate) prop: Rc<Property<T>>,
}

impl<T: 'static> SqFunction for Setter<T> {
	fn invoke(&mut self, vm: &SQVM) -> squirrel_sys::SQInteger {
		match unsafe { payload_ptr::<T>(vm, 1) } {
			Some(payload) => match (self.prop.set)(unsafe { &mut (*payload).value }, vm, 2) {
				Ok(()) => 0,
				Err(error) => vm.throw_error(&error),
			},
			None => vm.throw_error("the setter's receiver is not an instance of this native class"),
		}
	}
}

/// A [`Getter`] as the VM calls it directly (`sq_setnativeaccessor`): `ud` is its
/// property, `typetag` and `up` the instance's. Pushes the value, or declines (0) for an
/// instance without a `T` payload, whose call raises the error.
pub(crate) unsafe extern "C" fn get_direct<T: 'static>(
	vm: squirrel_sys::HSQUIRRELVM,
	ud: squirrel_sys::SQUserPointer,
	typetag: squirrel_sys::SQUserPointer,
	up: squirrel_sys::SQUserPointer,
) -> squirrel_sys::SQInteger {
	let Some(payload) = (unsafe { direct_payload::<T>(typetag, up) }) else { return 0 };
	let vm = unsafe { SQVM::from_raw(vm) };
	let prop = unsafe { &*(ud as *const Property<T>) };
	(prop.get)(unsafe { &(*payload).value }, &vm); // pushes exactly one value
	1
}

/// A [`Setter`] as the VM calls it directly, with the value on top of the stack. Declines
/// (0) where the call would raise: no `T` payload, or a value the property can't take.
pub(crate) unsafe extern "C" fn set_direct<T: 'static>(
	vm: squirrel_sys::HSQUIRRELVM,
	ud: squirrel_sys::SQUserPointer,
	typetag: squirrel_sys::SQUserPointer,
	up: squirrel_sys::SQUserPointer,
) -> squirrel_sys::SQInteger {
	let Some(payload) = (unsafe { direct_payload::<T>(typetag, up) }) else { return 0 };
	let vm = unsafe { SQVM::from_raw(vm) };
	let prop = unsafe { &*(ud as *const Property<T>) };
	// A scalar setter fails only reading the value, before it writes anything.
	match (prop.set)(unsafe { &mut (*payload).value }, &vm, -1) {
		Ok(()) => 1,
		Err(_) => 0,
	}
}

/// The instance's payload, as [`payload_ptr`] finds it, from its typetag and user pointer.
unsafe fn direct_payload<T: 'static>(
	typetag: squirrel_sys::SQUserPointer,
	up: squirrel_sys::SQUserPointer,
) -> Option<*mut super::payload::NativePayload<T>> {
	if typetag != super::payload::native_payload_tag() || up.is_null() {
		return None;
	}
	unsafe { super::payload::payload_of::<T>(up) }
}

/// What the accessor `table` holds under the key in stack slot 2, pushed, or `false`
/// (nothing pushed) if it holds nothing.
fn push_accessor(vm: &SQVM, table: &SqStrongRef) -> bool {
	let ptr = vm.as_ptr();
	table.push(vm);
	unsafe { squirrel_sys::sq_push(ptr, 2) };
	if unsafe { squirrel_sys::sq_rawget(ptr, -2) } < 0 {
		unsafe { squirrel_sys::sq_pop(ptr, 1) };
		return false;
	}
	true
}

/// No accessor for the key: throw `null`, as Sqrat does. The VM takes that as a clean
/// miss and goes on to the instance's default delegate (`weakref`, `getclass`, …),
/// then raises its own "the index '…' does not exist".
fn not_found(vm: &SQVM) -> squirrel_sys::SQInteger {
	let ptr = vm.as_ptr();
	unsafe {
		squirrel_sys::sq_pushnull(ptr);
		squirrel_sys::sq_throwobject(ptr)
	}
}

/// Whether the value on top of the stack is the object with identity `ours`.
fn top_is(vm: &SQVM, ours: i64) -> bool {
	let mut top = std::mem::MaybeUninit::<squirrel_sys::SQObject>::uninit();
	// SAFETY: the stack holds a value on top, so `sq_getstackobj` initializes `top`.
	if unsafe { squirrel_sys::sq_getstackobj(vm.as_ptr(), -1, top.as_mut_ptr()) } < 0 {
		return false;
	}
	unsafe { crate::SqObject::from_raw(top.assume_init()) }.identity() == ours
}

/// FNV-1a: much cheaper than the default SipHash for short property names (no
/// HashDoS concern: the keys are the class's own field names).
#[derive(Default)]
pub(crate) struct NameHasher(u64);

impl Hasher for NameHasher {
	fn write(&mut self, bytes: &[u8]) {
		let mut h = if self.0 == 0 {
			0xcbf2_9ce4_8422_2325
		} else {
			self.0
		};
		for &b in bytes {
			h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
		}
		self.0 = h;
	}

	fn finish(&self) -> u64 {
		self.0
	}
}

/// The class's `_get(key)` metamethod: call `__getTable[key]` with the instance as
/// `this`, or the Rust getter directly while that is still the class's own.
pub(crate) struct GetMeta<T> {
	pub(crate) props: PropertyTable<T>,
	/// The class's `__getTable`.
	pub(crate) table: SqStrongRef,
}

impl<T: 'static> SqFunction for GetMeta<T> {
	fn invoke(&mut self, vm: &SQVM) -> squirrel_sys::SQInteger {
		// Stack: [this(instance)=1, key=2, ...]. The key borrows the interned string in
		// slot 2, which stays put for the call.
		let Some(key) = vm.get_bytes_ref(2) else {
			return vm.throw_error("_get: property name must be a string");
		};
		let ptr = vm.as_ptr();
		// [.., table, accessor]
		if !push_accessor(vm, &self.table) {
			return not_found(vm);
		}
		if let Some(slot) = self.props.get(key)
			&& top_is(vm, slot.getter_id)
		{
			unsafe { squirrel_sys::sq_pop(ptr, 2) };
			let Some(payload) = (unsafe { payload_ptr::<T>(vm, 1) }) else {
				return vm.throw_error("_get: receiver is not an instance of this native class");
			};
			(slot.prop.get)(unsafe { &(*payload).value }, vm); // pushes exactly one value
			return 1;
		}
		// A mod's accessor: call it as Sqrat does, with the instance as `this`.
		unsafe { squirrel_sys::sq_push(ptr, 1) };
		if unsafe { squirrel_sys::sq_call(ptr, 1, 1, 1) } < 0 {
			return -1;
		}
		1
	}
}

/// The class's `_set(key, value)` metamethod: call `__setTable[key]` with the instance
/// as `this` and the value, or the Rust setter directly while that is still the class's
/// own.
pub(crate) struct SetMeta<T> {
	pub(crate) props: PropertyTable<T>,
	/// The class's `__setTable`.
	pub(crate) table: SqStrongRef,
}

impl<T: 'static> SqFunction for SetMeta<T> {
	fn invoke(&mut self, vm: &SQVM) -> squirrel_sys::SQInteger {
		// Stack: [this=1, key=2, value=3, ...]. The key borrows slot 2 (see `GetMeta`).
		let Some(key) = vm.get_bytes_ref(2) else {
			return vm.throw_error("_set: property name must be a string");
		};
		let ptr = vm.as_ptr();
		// No setter, a read-only property's included: a miss, as Sqrat's (verified in the
		// original: writing a sprite's `Finished` raises "the index 'Finished' does not exist").
		if !push_accessor(vm, &self.table) {
			return not_found(vm);
		}
		if let Some(slot) = self.props.get(key)
			&& slot.setter_id.is_some_and(|ours| top_is(vm, ours))
		{
			unsafe { squirrel_sys::sq_pop(ptr, 2) };
			let Some(payload) = (unsafe { payload_ptr::<T>(vm, 1) }) else {
				return vm.throw_error("_set: receiver is not an instance of this native class");
			};
			return match (slot.prop.set)(unsafe { &mut (*payload).value }, vm, 3) {
				// The VM ignores `_set`'s return value (the result of `a.x = v` comes
				// from the value operand), so pushing it back is a harmless convention.
				Ok(()) => {
					unsafe { squirrel_sys::sq_push(ptr, 3) };
					1
				}
				Err(error) => vm.throw_error(&error),
			};
		}
		// A mod's accessor: call it as Sqrat does, with the instance and the value.
		unsafe {
			squirrel_sys::sq_push(ptr, 1);
			squirrel_sys::sq_push(ptr, 3);
		}
		if unsafe { squirrel_sys::sq_call(ptr, 2, 0, 1) } < 0 {
			return -1;
		}
		unsafe { squirrel_sys::sq_push(ptr, 3) };
		1
	}
}
