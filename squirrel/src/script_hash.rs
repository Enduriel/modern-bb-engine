//! [`ScriptHash`]: a native-fn argument yielding the hash the original engine's
//! `toHash(x)` global returns for it.
//!
//! The original's `toHash` (registered at `0x5398b5`, body `0x53add0`, the switch at
//! `0x5373d0`) is Squirrel 3.0's `HashObj` in its 32-bit
//! build, with a single-precision float:
//!
//! * a string: its content hash (`SQString::_hash`, Squirrel 3.0's string hash, which
//!   our VM uses too), as a 32-bit script integer, so equal strings hash equally in
//!   every run (Reforged seeds its "fixed" rolls with it);
//! * an integer or a bool: the value (a bool as 0 or 1);
//! * a float: truncated toward zero, as `cvttss2si` of the float as an `f32` (so a value
//!   outside the 32-bit range, or NaN, is `-2147483648`);
//! * anything else: the object's address shifted right by 3 (null: 0). Only identity is
//!   observable there, which our addresses give too.

use crate::function::SqArg;
use crate::{SQ_OK, SQVM, SqObject};

/// The hash of the argument at this position, as the original's `toHash` (see [the
/// module docs](self)).
pub struct ScriptHash(pub i64);

impl SqArg for ScriptHash {
	type Item<'a> = ScriptHash;

	fn push_typemask(mask: &mut String) {
		mask.push('.');
	}

	fn from_sq<'a>(vm: &'a SQVM, idx: i64) -> Result<ScriptHash, String> {
		let mut raw = std::mem::MaybeUninit::<squirrel_sys::SQObject>::uninit();
		// SAFETY: a valid stack index is handed in by the call machinery; on success
		// `sq_getstackobj` fully initializes the object.
		if unsafe { squirrel_sys::sq_getstackobj(vm.as_ptr(), idx, raw.as_mut_ptr()) } != SQ_OK {
			return Err("toHash: could not read the argument".to_string());
		}
		let hash = match unsafe { SqObject::from_raw(raw.assume_init()) } {
			// The VM's string hash is Squirrel 3.0's 32-bit one (see `sqstring.h`).
			SqObject::String(_) => (unsafe { squirrel_sys::sq_gethash(vm.as_ptr(), idx) }) as u32 as i32 as i64,
			SqObject::Integer(n) => n,
			SqObject::Bool(b) => b as i64,
			SqObject::Float(f) => cvttss2si(f as f32),
			SqObject::Null => 0,
			other => other.identity() >> 3,
		};
		Ok(ScriptHash(hash))
	}
}

/// `cvttss2si`: truncate toward zero; NaN and values outside `i32` give `i32::MIN`.
fn cvttss2si(f: f32) -> i64 {
	let t = f.trunc();
	if t.is_nan() || t < i32::MIN as f32 || t >= 2147483648.0 {
		i32::MIN as i64
	} else {
		t as i32 as i64
	}
}

#[cfg(test)]
mod tests {
	use super::cvttss2si;

	#[test]
	fn floats_truncate_as_single_precision() {
		assert_eq!(cvttss2si(5.7), 5);
		assert_eq!(cvttss2si(-5.7), -5);
		assert_eq!(cvttss2si(16777217.0f64 as f32), 16777216);
		assert_eq!(cvttss2si(1e10), i32::MIN as i64);
		assert_eq!(cvttss2si(-1e10), i32::MIN as i64);
		assert_eq!(cvttss2si(f32::NAN), i32::MIN as i64);
	}
}
