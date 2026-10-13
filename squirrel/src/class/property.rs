//! Native properties and the `_get` / `_set` metamethods that back them.
//!
//! A native class declares no real Squirrel members; instead every field access
//! `inst.X` (and `inst.X = v`) misses and routes to the class's `_get` / `_set`
//! metamethods. Those read the instance's Rust payload and dispatch on the field
//! name to a registered getter/setter — so `R`/`G`/`B`/`A`/`RGBA` on a `color`
//! can all be views over one `u32`, staying consistent.

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
	fn read_field(vm: &SQVM, idx: i64) -> Option<f64> {
		vm.get_float(idx).ok()
	}
}

impl FieldValue for i64 {
	fn push_field(self, vm: &SQVM) {
		vm.push_integer(self);
	}
	fn read_field(vm: &SQVM, idx: i64) -> Option<i64> {
		vm.get_integer(idx).ok()
	}
}

impl FieldValue for bool {
	fn push_field(self, vm: &SQVM) {
		vm.push_bool(self);
	}
	fn read_field(vm: &SQVM, idx: i64) -> Option<bool> {
		vm.get_bool(idx).ok()
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

/// A single named property: a getter that pushes the field and a setter that
/// reads a value off the stack and writes the field. Both are type-erased over
/// the field's scalar type.
pub(crate) struct Property<T> {
	#[allow(clippy::type_complexity)]
	pub(crate) get: Box<dyn Fn(&T, &SQVM)>,
	#[allow(clippy::type_complexity)]
	pub(crate) set: Box<dyn Fn(&mut T, &SQVM, i64) -> Result<(), String>>,
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
		}
	}

	/// A write-ignored, read-null property: assignment accepts (and discards) any value,
	/// reading yields `null`. For object-valued fields a headless stub doesn't model —
	/// e.g. a render `Color` a script sets but nothing logic-relevant reads back.
	pub(crate) fn new_noop() -> Self {
		Property {
			get: Box::new(|_t, vm| vm.push_null()),
			set: Box::new(|_t, _vm, _idx| Ok(())),
		}
	}
}

/// Shared, per-class property table, captured by both metamethods. Keyed by the name's
/// bytes, so a lookup needs no UTF-8 check (a non-UTF-8 key simply matches nothing),
/// and hashed with [`NameHasher`]: every native field access hashes its name, and these
/// are short.
pub(crate) type PropertyTable<T> =
	Rc<HashMap<Box<[u8]>, Property<T>, BuildHasherDefault<NameHasher>>>;

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

/// The class's `_get(key)` metamethod: read the payload, dispatch on `key`.
pub(crate) struct GetMeta<T> {
	pub(crate) props: PropertyTable<T>,
}

impl<T: 'static> SqFunction for GetMeta<T> {
	fn invoke(&mut self, vm: &SQVM) -> squirrel_sys::SQInteger {
		// Stack: [this(instance)=1, key=2, ...]. The key borrows the interned string in
		// slot 2, which stays put for the call; it is only used before anything is
		// pushed.
		let Some(key) = vm.get_bytes_ref(2) else {
			return vm.throw_error("_get: property name must be a string");
		};
		let payload = match unsafe { payload_ptr::<T>(vm, 1) } {
			Some(payload) => payload,
			None => {
				return vm.throw_error("_get: receiver is not an instance of this native class");
			}
		};
		match self.props.get(key) {
			Some(property) => {
				let value: &T = unsafe { &(*payload).value };
				(property.get)(value, vm); // pushes exactly one value
				1
			}
			None => vm.throw_error(&format!(
				"the index '{}' does not exist",
				String::from_utf8_lossy(key)
			)),
		}
	}
}

/// The class's `_set(key, value)` metamethod: read the payload, write the field.
pub(crate) struct SetMeta<T> {
	pub(crate) props: PropertyTable<T>,
}

impl<T: 'static> SqFunction for SetMeta<T> {
	fn invoke(&mut self, vm: &SQVM) -> squirrel_sys::SQInteger {
		// Stack: [this=1, key=2, value=3, ...]. The key borrows slot 2 (see `GetMeta`).
		let Some(key) = vm.get_bytes_ref(2) else {
			return vm.throw_error("_set: property name must be a string");
		};
		let payload = match unsafe { payload_ptr::<T>(vm, 1) } {
			Some(payload) => payload,
			None => {
				return vm.throw_error("_set: receiver is not an instance of this native class");
			}
		};
		match self.props.get(key) {
			Some(property) => {
				let value: &mut T = unsafe { &mut (*payload).value };
				match (property.set)(value, vm, 3) {
					// The VM ignores `_set`'s return value (the result of `a.x = v`
					// comes from the value operand), so pushing it back is a harmless
					// convention rather than a requirement.
					Ok(()) => {
						unsafe { squirrel_sys::sq_push(vm.as_ptr(), 3) };
						1
					}
					Err(error) => vm.throw_error(&error),
				}
			}
			None => vm.throw_error(&format!(
				"the index '{}' does not exist",
				String::from_utf8_lossy(key)
			)),
		}
	}
}
