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

	/// Read a native's integer argument the way the original's Sqrat bindings do: an
	/// integer as-is, a float truncated, and **any other value (null, bool, string,
	/// table, …) as `0`, without raising**.
	///
	/// BB script passes floats where a native wants an integer (a loop variable seeded
	/// from `_rect.Y + _rect.H * 0.2` stays a float across `++`), and it passes
	/// non-numbers too: vanilla's loot tooltip calls `Tactical.getEntityByID(null)`.
	/// The original's `Var<int>` (disassembled, e.g. the `getEntityByID` thunk at
	/// `0x4d8360`) inlines `sq_getinteger` and ignores its failure, so a non-number
	/// leaves the C++ local uninitialised: the native runs with stack garbage (in most
	/// thunks the VM pointer's bits; probed: `Math.abs(null)`, `Math.rand(true, true)`
	/// and `Math.min("5", "5")` all return the same 40976472, `getEntityByID(null)`
	/// returns null). No original native that goes through Sqrat raises on it. `0` is
	/// our deterministic stand-in (listed in docs/differences.md: the garbage can't be
	/// reproduced). The width stays 64-bit.
	pub(crate) fn get_int_lenient(&self, idx: i64) -> i64 {
		match self.get_type(idx) {
			SqObjectType::Integer => self.get_integer(idx).unwrap_or(0),
			SqObjectType::Float => self.get_float(idx).map_or(0, |f| f as i64),
			_ => 0,
		}
	}

	/// Read a native's float argument the way the original's Sqrat bindings do: a
	/// float as-is, an integer widened, and any other value as `0.0` without raising
	/// (the original reads uninitialised memory there, typically a pointer's bits as a
	/// tiny f32). The float counterpart to [`get_int_lenient`](Self::get_int_lenient).
	pub(crate) fn get_float_lenient(&self, idx: i64) -> f64 {
		match self.get_type(idx) {
			SqObjectType::Float => self.get_float(idx).unwrap_or(0.0),
			SqObjectType::Integer => self.get_integer(idx).map_or(0.0, |n| n as f64),
			_ => 0.0,
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
