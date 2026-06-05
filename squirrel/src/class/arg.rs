//! [`Native<T>`]: read a native-class instance argument back into its payload.

use super::payload::payload_ptr;
use crate::SQVM;
use crate::function::SqArg;

/// A native-class instance argument, yielding a clone of its Rust payload `T`.
///
/// Use it as a parameter type so a native function can accept one of its own
/// instances and recover the typed value — e.g. `getVecDistance(a: Native<Vec2f>,
/// b: Native<Vec2f>)`. The Squirrel type mask is `x` (any instance); the concrete
/// `T` is then checked by typetag + `TypeId`, erroring if it does not match.
pub struct Native<T>(pub T);

impl<T: Clone + 'static> SqArg for Native<T> {
	type Item<'a> = Native<T>;

	fn push_typemask(mask: &mut String) {
		mask.push('x');
	}

	fn from_sq<'a>(vm: &'a SQVM, idx: i64) -> Result<Native<T>, String> {
		match unsafe { payload_ptr::<T>(vm, idx) } {
			Some(payload) => Ok(Native(unsafe { (*payload).value.clone() })),
			None => Err("expected a native instance of the expected class".to_string()),
		}
	}
}
