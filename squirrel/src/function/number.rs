//! [`Number`]: a numeric argument that accepts an integer *or* a float.

use super::{IntoSqReturn, SqArg};
use crate::{SQVM, SqObjectType};

/// A numeric argument accepting either an integer or a float, yielded as `f64`.
///
/// Squirrel is loose with numbers — scripts pass integer literals to float
/// parameters freely (`createVec(0, -1)` alongside `createVec(0.0, -1.0)`). As an
/// argument it reads like an `f64` parameter: any value is accepted (mask `.`) and a
/// non-number reads as `0.0` without raising, as the original's Sqrat bindings do
/// (see `SQVM::get_float_lenient`).
pub struct Number(pub f64);

impl SqArg for Number {
	type Item<'a> = Number;

	fn push_typemask(mask: &mut String) {
		mask.push('.');
	}

	fn from_sq<'a>(vm: &'a SQVM, idx: i64) -> Result<Number, String> {
		Ok(Number(vm.get_float_lenient(idx)))
	}
}

/// As a container element ([`SqArray::to_vec`](crate::SqArray::to_vec),
/// [`SqTable::get`](crate::SqTable::get)), `Number` reads an integer *or* a float as
/// `f64`. A strict `f64` read skips integers, which silently shifts every index of a
/// mixed array like `[0, 10.0, 15.38]`.
impl crate::SqValue for Number {
	type Item<'a> = Number;
	fn push_value(self, vm: &SQVM) {
		vm.push_float(self.0);
	}
	fn read<'a>(vm: &'a SQVM, idx: i64) -> Option<Number> {
		// Strict, unlike the argument read: a non-number element is not a `Number`.
		match vm.get_type(idx) {
			SqObjectType::Integer => vm.get_integer(idx).ok().map(|n| Number(n as f64)),
			SqObjectType::Float => vm.get_float(idx).ok().map(Number),
			_ => None,
		}
	}
}

/// A number argument/return that *preserves* whether it was an integer or a float.
///
/// Unlike [`Number`] (which always yields `f64`), `DynNumber` remembers the kind,
/// so a function like `Math.abs` can return an integer for an integer input and a
/// float for a float input — matching how Squirrel scripts expect numeric type to
/// flow through. As an argument it accepts any value (mask `.`): a non-number reads
/// as `Int(0)` without raising, as the original's Sqrat bindings do (see
/// `SQVM::get_int_lenient`); as a return it pushes the matching type. Use [`as_i64`](Self::as_i64) / [`as_f64`](Self::as_f64)
/// to coerce when a fixed type is needed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DynNumber {
	Int(i64),
	Float(f64),
}

impl DynNumber {
	/// Coerce to `i64` (a float is truncated toward zero).
	pub fn as_i64(self) -> i64 {
		match self {
			DynNumber::Int(n) => n,
			DynNumber::Float(f) => f as i64,
		}
	}

	/// Coerce to `f64`.
	pub fn as_f64(self) -> f64 {
		match self {
			DynNumber::Int(n) => n as f64,
			DynNumber::Float(f) => f,
		}
	}
}

impl SqArg for DynNumber {
	type Item<'a> = DynNumber;

	fn push_typemask(mask: &mut String) {
		mask.push('.');
	}

	fn from_sq<'a>(vm: &'a SQVM, idx: i64) -> Result<DynNumber, String> {
		Ok(match vm.get_type(idx) {
			SqObjectType::Float => DynNumber::Float(vm.get_float_lenient(idx)),
			_ => DynNumber::Int(vm.get_int_lenient(idx)),
		})
	}
}

impl IntoSqReturn for DynNumber {
	fn push_return(self, vm: &SQVM) -> Result<i64, String> {
		match self {
			DynNumber::Int(n) => vm.push_integer(n),
			DynNumber::Float(f) => vm.push_float(f),
		}
		Ok(1)
	}
}
