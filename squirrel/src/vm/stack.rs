//! Typed reads and writes against the Squirrel value stack: the low-level
//! marshalling primitives every higher layer (argument extraction, return
//! pushing, the trampoline) builds on.
//!
//! These are deliberately `pub(crate)`: the raw stack is an implementation detail
//! the `squirrel` crate owns and abstracts over (typed `register`/`make_closure`
//! signatures, `ClassBuilder` properties/methods). Downstream crates must never
//! touch the stack directly — extend the typed abstractions here instead. (If you
//! are tempted to make one of these `pub`, that is the signal to add an
//! abstraction, not to widen the surface.)

use std::ffi::{CString, c_char};

use super::SQVM;
use crate::{SQ_OK, SqObjectType};

impl SQVM {
	/// Number of values currently on the stack. During a native call this
	/// includes `this` (index 1), the call arguments, and any bound free
	/// variables (which sit above the arguments).
	pub(crate) fn stack_top(&self) -> i64 {
		unsafe { squirrel_sys::sq_gettop(self.as_ptr()) }
	}

	pub(crate) fn get_type(&self, idx: i64) -> SqObjectType {
		// Never panic: this runs inside the `extern "C"` trampoline (via `Number`,
		// `This`, `SqRef`, …) where unwinding across the C frame is UB. A valid
		// stack index always yields a known `OT_*` tag, but an unrecognized tag
		// (corruption, or an out-of-range index) degrades to `Null`, which every
		// caller treats as "not the type I wanted" and rejects gracefully.
		SqObjectType::try_from(unsafe { squirrel_sys::sq_gettype(self.as_ptr(), idx) })
			.unwrap_or(SqObjectType::Null)
	}

	pub(crate) fn get_integer(&self, idx: i64) -> Result<i64, String> {
		let mut value: i64 = 0;
		let res = unsafe { squirrel_sys::sq_getinteger(self.as_ptr(), idx, &mut value) };
		if res != SQ_OK {
			return Err(format!("expected an integer at parameter {}", idx - 1));
		}
		Ok(value)
	}

	pub(crate) fn get_float(&self, idx: i64) -> Result<f64, String> {
		let mut value: f64 = 0.0;
		let res = unsafe { squirrel_sys::sq_getfloat(self.as_ptr(), idx, &mut value) };
		if res != SQ_OK {
			return Err(format!("expected a float at parameter {}", idx - 1));
		}
		Ok(value)
	}

	/// Read a numeric argument as an integer, accepting an integer *or* a float
	/// (truncating the float). BB script routinely passes floats where a native
	/// wants an integer — loop variables seeded from float expressions such as
	/// `_rect.Y + _rect.H * 0.2` stay floats across `++`, and the resulting
	/// coordinate flows into integer natives like `getTileSquare`. Squirrel's own
	/// array indexing and `sq_getinteger` coerce the same way; we mirror that here
	/// instead of relying on a permissive type mask. Errors if the value is neither
	/// an integer nor a float.
	pub(crate) fn get_int_coerced(&self, idx: i64) -> Result<i64, String> {
		match self.get_type(idx) {
			SqObjectType::Integer => self.get_integer(idx),
			SqObjectType::Float => Ok(self.get_float(idx)? as i64),
			other => Err(format!(
				"expected an integer or float at parameter {} (got {other:?})",
				idx - 1
			)),
		}
	}

	/// Read a numeric argument as a float, accepting a float *or* an integer
	/// (widening the integer). The float counterpart to [`get_int_coerced`].
	pub(crate) fn get_float_coerced(&self, idx: i64) -> Result<f64, String> {
		match self.get_type(idx) {
			SqObjectType::Float => self.get_float(idx),
			SqObjectType::Integer => Ok(self.get_integer(idx)? as f64),
			other => Err(format!(
				"expected an integer or float at parameter {} (got {other:?})",
				idx - 1
			)),
		}
	}

	pub(crate) fn get_bool(&self, idx: i64) -> Result<bool, String> {
		let mut value: squirrel_sys::SQBool = 0;
		let res = unsafe { squirrel_sys::sq_getbool(self.as_ptr(), idx, &mut value) };
		if res != SQ_OK {
			return Err(format!("expected a bool at parameter {}", idx - 1));
		}
		Ok(value != 0)
	}

	/// Borrow the string at `idx` directly from the stack — no copy.
	///
	/// The returned slice points into the (immutable, non-relocated) Squirrel
	/// string object, which stays alive on the stack for the whole native call,
	/// so the borrow is valid for as long as the `&self` borrow of the VM.
	pub(crate) fn get_str_ref(&self, idx: i64) -> Result<&str, String> {
		let bytes = self
			.get_bytes_ref(idx)
			.ok_or_else(|| format!("expected a string at parameter {}", idx - 1))?;
		std::str::from_utf8(bytes).map_err(|_| format!("parameter {} is not valid UTF-8", idx - 1))
	}

	/// The raw bytes of the string at `idx` (no UTF-8 check), borrowed like
	/// [`get_str_ref`](Self::get_str_ref); `None` if it is not a string.
	pub(crate) fn get_bytes_ref(&self, idx: i64) -> Option<&[u8]> {
		// `sq_getstringandsize` fetches pointer and length together, so they are
		// guaranteed to describe the same string object (one FFI crossing).
		let mut c_str_ptr: *const c_char = std::ptr::null();
		let mut size: squirrel_sys::SQInteger = 0;
		let res = unsafe {
			squirrel_sys::sq_getstringandsize(self.as_ptr(), idx, &mut c_str_ptr, &mut size)
		};
		// Defensive: a valid string never has a negative length, but guard the
		// cast to `usize` so a malformed size can't produce an oversized slice.
		if res != SQ_OK || size < 0 || c_str_ptr.is_null() {
			return None;
		}
		Some(unsafe { std::slice::from_raw_parts(c_str_ptr as *const u8, size as usize) })
	}

	pub(crate) fn get_string(&self, idx: i64) -> Result<String, String> {
		self.get_str_ref(idx).map(str::to_owned)
	}

	pub(crate) fn push_null(&self) {
		unsafe { squirrel_sys::sq_pushnull(self.as_ptr()) };
	}

	pub(crate) fn push_integer(&self, value: i64) {
		unsafe { squirrel_sys::sq_pushinteger(self.as_ptr(), value) };
	}

	pub(crate) fn push_float(&self, value: f64) {
		unsafe { squirrel_sys::sq_pushfloat(self.as_ptr(), value) };
	}

	pub(crate) fn push_bool(&self, value: bool) {
		unsafe { squirrel_sys::sq_pushbool(self.as_ptr(), value as squirrel_sys::SQBool) };
	}

	pub(crate) fn push_string(&self, value: &str) {
		self.push_bytes(value.as_bytes());
	}

	/// Push a string of raw bytes (Squirrel strings needn't be UTF-8).
	pub(crate) fn push_bytes(&self, value: &[u8]) {
		unsafe {
			squirrel_sys::sq_pushstring(
				self.as_ptr(),
				value.as_ptr() as *const c_char,
				value.len() as i64,
			)
		};
	}

	pub(crate) fn push_str(&mut self, s: &str) {
		unsafe {
			squirrel_sys::sq_pushstring(self.as_ptr(), s.as_ptr() as *const c_char, s.len() as i64);
		}
	}

	/// Raise a Squirrel exception with `message` and return the sentinel that a
	/// native closure must hand back so the VM unwinds the call.
	pub(crate) fn throw_error(&self, message: &str) -> squirrel_sys::SQInteger {
		// `sq_throwerror` copies the string, so a temporary `CString` is fine.
		// Fall back to a static message if the error text itself has a NUL byte.
		let c_message = std::ffi::CString::new(message)
			.unwrap_or_else(|_| CString::new("native function error").unwrap());
		unsafe { squirrel_sys::sq_throwerror(self.as_ptr(), c_message.as_ptr()) }
	}
}
