//! [`Number`]: a numeric argument that accepts an integer *or* a float.

use super::{IntoSqReturn, SqArg};
use crate::{SQVM, SqObjectType};

/// A numeric argument accepting either an integer or a float, yielded as `f64`.
///
/// Squirrel is loose with numbers — scripts pass integer literals to float
/// parameters freely (`createVec(0, -1)` alongside `createVec(0.0, -1.0)`). A
/// plain `f64` parameter uses the strict `f` type mask and would reject the
/// integer form; `Number` uses the `n` (number) mask and converts either to
/// `f64`, matching that leniency.
pub struct Number(pub f64);

impl SqArg for Number {
	type Item<'a> = Number;

	fn push_typemask(mask: &mut String) {
		mask.push('n');
	}

	fn from_sq<'a>(vm: &'a SQVM, idx: i64) -> Result<Number, String> {
		match vm.get_type(idx) {
			SqObjectType::Integer => vm.get_integer(idx).map(|n| Number(n as f64)),
			SqObjectType::Float => vm.get_float(idx).map(Number),
			other => Err(format!("expected a number, got {other:?}")),
		}
	}
}

/// A number argument/return that *preserves* whether it was an integer or a float.
///
/// Unlike [`Number`] (which always yields `f64`), `DynNumber` remembers the kind,
/// so a function like `Math.abs` can return an integer for an integer input and a
/// float for a float input — matching how Squirrel scripts expect numeric type to
/// flow through. As an argument it accepts either (mask `n`); as a return it
/// pushes the matching type. Use [`as_i64`](Self::as_i64) / [`as_f64`](Self::as_f64)
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
		mask.push('n');
	}

	fn from_sq<'a>(vm: &'a SQVM, idx: i64) -> Result<DynNumber, String> {
		match vm.get_type(idx) {
			SqObjectType::Integer => vm.get_integer(idx).map(DynNumber::Int),
			SqObjectType::Float => vm.get_float(idx).map(DynNumber::Float),
			other => Err(format!("expected a number, got {other:?}")),
		}
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
