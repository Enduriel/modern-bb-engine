//! [`SqValue`]: conversion for the *elements* of container wrappers like
//! [`SqArray`](super::SqArray).
//!
//! The associated [`SqValue::Item`] is what a *read* hands back, parameterized by
//! the call's lifetime: for owned types (scalars, `String`) it is `Self`; for a
//! borrowed handle like [`SqArray`](super::SqArray) it is `SqArray<'a>` — a
//! handle into the live container, no copy. Reads thread the VM borrow into
//! `Item<'a>`, which is what lets [`SqArray::get`](super::SqArray::get) return a
//! *nested* array handle that cannot escape the borrow (same model as
//! [`This`](crate::This)).
//!
//! `push_value` is named to avoid colliding with the inherent
//! [`SqArray::push`](super::SqArray::push) once `SqArray` itself implements this
//! trait (the nesting case).

use crate::{SQVM, SqObjectType};

/// A value that can be pushed onto, and read back from, the Squirrel stack.
pub trait SqValue: Sized {
	/// What a read at a given index produces, borrowing the VM for `'a`. Owned
	/// types set this to `Self`; borrowed handles set it to their `'a` form.
	type Item<'a>;

	/// Push `self` onto the stack as exactly one value.
	fn push_value(self, vm: &SQVM);

	/// Read the value at stack `idx`, or `None` if it is not the expected type.
	fn read<'a>(vm: &'a SQVM, idx: i64) -> Option<Self::Item<'a>>;
}

impl SqValue for i64 {
	type Item<'a> = i64;
	fn push_value(self, vm: &SQVM) {
		vm.push_integer(self);
	}
	fn read<'a>(vm: &'a SQVM, idx: i64) -> Option<i64> {
		vm.get_integer(idx).ok()
	}
}

impl SqValue for f64 {
	type Item<'a> = f64;
	fn push_value(self, vm: &SQVM) {
		vm.push_float(self);
	}
	fn read<'a>(vm: &'a SQVM, idx: i64) -> Option<f64> {
		vm.get_float(idx).ok()
	}
}

impl SqValue for bool {
	type Item<'a> = bool;
	fn push_value(self, vm: &SQVM) {
		vm.push_bool(self);
	}
	fn read<'a>(vm: &'a SQVM, idx: i64) -> Option<bool> {
		vm.get_bool(idx).ok()
	}
}

impl SqValue for String {
	type Item<'a> = String;
	fn push_value(self, vm: &SQVM) {
		vm.push_string(&self);
	}
	fn read<'a>(vm: &'a SQVM, idx: i64) -> Option<String> {
		vm.get_string(idx).ok()
	}
}

/// `None` round-trips as Squirrel `null`.
impl<T: SqValue> SqValue for Option<T> {
	type Item<'a> = Option<T::Item<'a>>;
	fn push_value(self, vm: &SQVM) {
		match self {
			Some(value) => value.push_value(vm),
			None => vm.push_null(),
		}
	}
	fn read<'a>(vm: &'a SQVM, idx: i64) -> Option<Option<T::Item<'a>>> {
		if matches!(vm.get_type(idx), SqObjectType::Null) {
			Some(None)
		} else {
			T::read(vm, idx).map(Some)
		}
	}
}
