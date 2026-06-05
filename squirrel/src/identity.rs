//! [`Identity`]: a native-fn argument yielding any object's stable identity hash.
//!
//! This backs Battle Brothers' `toHash(x)` global, which scripts use to derive a
//! unique-per-object id (e.g. `item.getInstanceID() == toHash(this).tostring()`).
//! The exact engine formula is unknown — the mod-kit author noted only "some sort
//! of hashing function, but I couldn't immediately identify it" — but the
//! observable contract is "a stable integer that is unique per live object", which
//! the object's address satisfies for reference types (Squirrel never relocates a
//! live object: it is plain reference-counting, no compaction). So:
//!
//!   * reference types (instance/table/array/string/closure/…) → their pointer,
//!   * scalars (integer/bool) → the value, float → its bit pattern, null → 0.
//!
//! Interned strings share one [`SQString`](squirrel_sys::SQString), so equal
//! strings hash equally; identity is otherwise per-object, not per-value.

use crate::function::SqArg;
use crate::{SQ_OK, SQVM, SqObject};

/// A native-fn argument capturing the identity hash of the object passed at that
/// position. Use it as a parameter type to accept a value of *any* type and read
/// its identity — see [the module docs](self).
pub struct Identity(pub i64);

impl SqArg for Identity {
	type Item<'a> = Identity;

	fn push_typemask(mask: &mut String) {
		// Any type — `toHash` is called on instances, strings, and more.
		mask.push('.');
	}

	fn from_sq<'a>(vm: &'a SQVM, idx: i64) -> Result<Identity, String> {
		let mut raw = std::mem::MaybeUninit::<squirrel_sys::SQObject>::uninit();
		// SAFETY: a valid stack index is handed in by the call machinery; on success
		// `sq_getstackobj` fully initializes the object.
		let res = unsafe { squirrel_sys::sq_getstackobj(vm.as_ptr(), idx, raw.as_mut_ptr()) };
		if res != SQ_OK {
			return Err("toHash: could not read the argument".to_string());
		}
		let obj = unsafe { SqObject::from_raw(raw.assume_init()) };
		Ok(Identity(obj.identity()))
	}
}

impl SQVM {
	/// The pointer-identity of the receiver (`this`, stack index 1) of the *current
	/// native call* — the same stable integer [`Identity`] yields, but for the
	/// receiver rather than an argument. Returns `0` if there is no valid receiver.
	///
	/// This is the abstraction native **method injection** uses to recover which
	/// script object a shared native method was invoked on (the state stack, and
	/// later the World/Tactical entity methods that index a Rust-owned arena). It
	/// returns a plain integer — no stack handle is exposed.
	pub fn this_identity(&self) -> i64 {
		let mut raw = std::mem::MaybeUninit::<squirrel_sys::SQObject>::uninit();
		if unsafe { squirrel_sys::sq_getstackobj(self.as_ptr(), 1, raw.as_mut_ptr()) } != SQ_OK {
			return 0;
		}
		// SAFETY: `sq_getstackobj` succeeded, so the object is initialized; `from_raw`
		// falls back gracefully only on a recognized tag (a malformed `_type` panics,
		// matching the rest of the crate's contract for a VM-produced object).
		unsafe { SqObject::from_raw(raw.assume_init()) }.identity()
	}
}
