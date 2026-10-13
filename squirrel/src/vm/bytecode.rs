//! Loading a serialized (compiled) closure via `sq_readclosure`, and writing one via
//! `sq_writeclosure`.
//!
//! The bytes must be in THIS VM's bytecode format. Game `.cnut` files are an older
//! Squirrel version (32-bit) and must be transcoded to this format first (that
//! lives in the `bb-engine` crate); this layer is just the version-agnostic
//! "read a closure stream from memory" primitive.

use std::ffi::c_void;

use super::SQVM;
use crate::{SQ_OK, SqStrongRef};

/// Cursor over an in-memory byte slice, handed to `sq_readclosure` as its reader.
struct ByteReader {
	data: *const u8,
	len: usize,
	pos: usize,
}

/// `SQREADFUNC`: copy up to `size` bytes into `dest`, returning how many were
/// available. A short read makes Squirrel's `SafeRead` report a truncated stream.
unsafe extern "C" fn read_mem(
	up: squirrel_sys::SQUserPointer,
	dest: squirrel_sys::SQUserPointer,
	size: squirrel_sys::SQInteger,
) -> squirrel_sys::SQInteger {
	let reader = unsafe { &mut *(up as *mut ByteReader) };
	let want = size.max(0) as usize;
	let n = want.min(reader.len - reader.pos);
	if n > 0 {
		unsafe { std::ptr::copy_nonoverlapping(reader.data.add(reader.pos), dest as *mut u8, n) };
		reader.pos += n;
	}
	n as squirrel_sys::SQInteger
}

/// `SQWRITEFUNC`: append `size` bytes from `src` to the `Vec<u8>` behind `up`.
unsafe extern "C" fn write_mem(
	up: squirrel_sys::SQUserPointer,
	src: squirrel_sys::SQUserPointer,
	size: squirrel_sys::SQInteger,
) -> squirrel_sys::SQInteger {
	let out = unsafe { &mut *(up as *mut Vec<u8>) };
	let n = size.max(0) as usize;
	out.extend_from_slice(unsafe { std::slice::from_raw_parts(src as *const u8, n) });
	size
}

impl SqStrongRef {
	/// Serialize this script closure in this VM's bytecode format: the inverse of
	/// [`SQVM::load_closure`]. Fails for a native closure, or one with bound free
	/// variables (as `sq_writeclosure` does).
	pub fn write_closure(&self, vm: &SQVM) -> Result<Vec<u8>, String> {
		let ptr = vm.as_ptr();
		let mut out: Vec<u8> = Vec::new();
		self.push(vm); // [closure]
		unsafe {
			let up = &mut out as *mut Vec<u8> as *mut c_void;
			let ok = squirrel_sys::sq_writeclosure(ptr, Some(write_mem), up) == SQ_OK;
			if !ok {
				squirrel_sys::sq_getlasterror(ptr); // [closure, error]
				let message = vm
					.get_str_ref(-1)
					.map(str::to_owned)
					.unwrap_or_else(|_| "failed to write closure".to_string());
				squirrel_sys::sq_pop(ptr, 2);
				squirrel_sys::sq_reseterror(ptr);
				return Err(message);
			}
			squirrel_sys::sq_pop(ptr, 1);
		}
		Ok(out)
	}
}

impl SQVM {
	/// Whether the compiler emits debug info (`sq_enabledebuginfo`, or `enabledebuginfo()`
	/// from a script). It changes the bytecode a compile produces.
	pub fn debug_info_enabled(&self) -> bool {
		unsafe { squirrel_sys::sq_getdebuginfo(self.as_ptr()) != 0 }
	}

	/// Load a serialized closure (as produced by `sq_writeclosure`, in this VM's
	/// format) from `bytes`, returning it as an owned reference. Run it with
	/// [`SqStrongRef::call`] (which binds `this` to the root table).
	pub fn load_closure(&self, bytes: &[u8]) -> Result<SqStrongRef, String> {
		let ptr = self.as_ptr();
		let mut reader = ByteReader {
			data: bytes.as_ptr(),
			len: bytes.len(),
			pos: 0,
		};
		unsafe {
			let up = &mut reader as *mut ByteReader as *mut c_void;
			if squirrel_sys::sq_readclosure(ptr, Some(read_mem), up) != SQ_OK {
				squirrel_sys::sq_getlasterror(ptr); // [.., error]
				let message = self
					.get_str_ref(-1)
					.map(str::to_owned)
					.unwrap_or_else(|_| "failed to read closure".to_string());
				squirrel_sys::sq_pop(ptr, 1);
				squirrel_sys::sq_reseterror(ptr);
				return Err(message);
			}
			// On success the closure is on top of the stack.
			let closure = SqStrongRef::from_stack(self, -1);
			squirrel_sys::sq_pop(ptr, 1);
			Ok(closure)
		}
	}
}
