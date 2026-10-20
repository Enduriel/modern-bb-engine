//! [`SqArg`]: how each closure parameter type is read off the Squirrel stack and
//! what it contributes to the function's type mask.

use crate::{SQVM, SqArray, SqBytes, SqObjectType, SqRef, SqTable, SqText, SqValue};

/// A Rust type that can be read from a Squirrel call argument.
///
/// The associated [`SqArg::Item`] is the value the closure actually receives,
/// parameterized by the call's lifetime: for owned types it is `Self`, but for a
/// borrowed type like `&str` it is `&'a str` — a slice pointing straight into the
/// argument on the stack, with no copy. The closure-registration machinery uses
/// `Item<'a>` to hand each call's arguments a borrow tied to that call, which is
/// why borrowed args cannot escape it (same guarantee as [`This`](super::This)).
pub trait SqArg {
	/// The value handed to the closure for a call lasting `'a`. `Self` for owned
	/// types; a borrow of the stack (e.g. `&'a str`) for borrowed ones.
	type Item<'a>;

	/// Append this type's type-mask character(s), e.g. `"i"`, `"s"`, `"i|o"`.
	fn push_typemask(mask: &mut String);

	/// Read the value at stack `idx`, borrowing the VM for the call's lifetime.
	fn from_sq<'a>(vm: &'a SQVM, idx: i64) -> Result<Self::Item<'a>, String>;
}

impl SqArg for i64 {
	type Item<'a> = i64;
	fn push_typemask(mask: &mut String) {
		// Accept any value at the VM level (like `SqRef`); `from_sq` converts it as the
		// original's Sqrat bindings do (float -> truncated, non-number -> 0, never an
		// error). A strict `'i'` mask would reject calls the original accepts.
		mask.push('.');
	}
	fn from_sq<'a>(vm: &'a SQVM, idx: i64) -> Result<i64, String> {
		Ok(vm.get_int_lenient(idx))
	}
}

impl SqArg for f64 {
	type Item<'a> = f64;
	fn push_typemask(mask: &mut String) {
		// As with `i64`: accept any value, then convert in `from_sq` (int -> widened,
		// non-number -> 0.0), so an integer argument satisfies a float parameter.
		mask.push('.');
	}
	fn from_sq<'a>(vm: &'a SQVM, idx: i64) -> Result<f64, String> {
		Ok(vm.get_float_lenient(idx))
	}
}

impl SqArg for bool {
	type Item<'a> = bool;
	fn push_typemask(mask: &mut String) {
		mask.push('b');
	}
	fn from_sq<'a>(vm: &'a SQVM, idx: i64) -> Result<bool, String> {
		vm.get_bool(idx)
	}
}

impl SqArg for String {
	type Item<'a> = String;
	fn push_typemask(mask: &mut String) {
		mask.push('s');
	}
	fn from_sq<'a>(vm: &'a SQVM, idx: i64) -> Result<String, String> {
		vm.get_string(idx)
	}
}

/// A borrowed string argument: the slice points into the stack with no copy.
/// Prefer this over `String` to avoid an allocation per call when you only need
/// to read the argument.
impl SqArg for &str {
	type Item<'a> = &'a str;
	fn push_typemask(mask: &mut String) {
		mask.push('s');
	}
	fn from_sq<'a>(vm: &'a SQVM, idx: i64) -> Result<&'a str, String> {
		vm.get_str_ref(idx)
	}
}

/// A string argument as its raw bytes, UTF-8 or not (see [`SqBytes`]).
impl SqArg for SqBytes {
	type Item<'a> = SqBytes;
	fn push_typemask(mask: &mut String) {
		mask.push('s');
	}
	fn from_sq<'a>(vm: &'a SQVM, idx: i64) -> Result<SqBytes, String> {
		vm.get_bytes_ref(idx)
			.map(|b| SqBytes(b.to_vec()))
			.ok_or_else(|| format!("expected a string at parameter {}", idx - 1))
	}
}

/// A table argument, received as a borrowed [`SqTable`] handle into the live
/// argument (same borrowed model as `&str`: it cannot escape the call).
impl SqArg for SqTable<'_> {
	type Item<'a> = SqTable<'a>;
	fn push_typemask(mask: &mut String) {
		mask.push('t');
	}
	fn from_sq<'a>(vm: &'a SQVM, idx: i64) -> Result<SqTable<'a>, String> {
		SqTable::from_stack(vm, idx).ok_or_else(|| "expected a table".to_string())
	}
}

/// An array argument, received as a borrowed [`SqArray`] handle into the live
/// argument.
impl SqArg for SqArray<'_> {
	type Item<'a> = SqArray<'a>;
	fn push_typemask(mask: &mut String) {
		mask.push('a');
	}
	fn from_sq<'a>(vm: &'a SQVM, idx: i64) -> Result<SqArray<'a>, String> {
		SqArray::from_stack(vm, idx).ok_or_else(|| "expected an array".to_string())
	}
}

/// An any-type argument, received as a matchable [`SqRef`] (type mask `.`).
/// Use it for a native function that accepts a value of any type (e.g. a
/// pass-through / variadic-any parameter).
impl SqArg for SqRef<'_> {
	type Item<'a> = SqRef<'a>;
	fn push_typemask(mask: &mut String) {
		mask.push('.');
	}
	fn from_sq<'a>(vm: &'a SQVM, idx: i64) -> Result<SqRef<'a>, String> {
		SqRef::read(vm, idx).ok_or_else(|| "could not read argument".to_string())
	}
}

/// Any argument as `tostring` writes it (see [`SqText`]; type mask `.`).
impl SqArg for SqText {
	type Item<'a> = SqText;
	fn push_typemask(mask: &mut String) {
		mask.push('.');
	}
	fn from_sq<'a>(vm: &'a SQVM, idx: i64) -> Result<SqText, String> {
		Ok(SqText::of(vm, idx))
	}
}

/// An optional argument: accepts the wrapped type or `null`.
impl<T: SqArg> SqArg for Option<T> {
	type Item<'a> = Option<T::Item<'a>>;
	fn push_typemask(mask: &mut String) {
		T::push_typemask(mask);
		mask.push_str("|o");
	}
	fn from_sq<'a>(vm: &'a SQVM, idx: i64) -> Result<Option<T::Item<'a>>, String> {
		if matches!(vm.get_type(idx), SqObjectType::Null) {
			Ok(None)
		} else {
			T::from_sq(vm, idx).map(Some)
		}
	}
}
