//! The [`SQVM`] handle: owns the underlying `HSQUIRRELVM`, manages its
//! lifecycle, and installs the default print / error / compiler-error callbacks.
//!
//! The VM's behaviour is split across focused submodules:
//!   * [`stack`]  — typed reads/writes against the Squirrel value stack
//!   * [`script`] — compiling/running scripts and allocating Rust userdata
//!   * [`print`]  — the print / error / compile-error handler dispatch

mod bytecode;
mod print;
mod runtime_error;
mod script;
mod stack;
#[cfg(test)]
mod float_format_tests;
#[cfg(test)]
mod stdlib_parity_tests;

pub use print::CompileError;
pub use runtime_error::{ErrorValue, Frame, Local, RuntimeError};

use print::{VmContext, compile_error_trampoline, error_trampoline, print_trampoline};

pub struct SQVM {
	ptr: squirrel_sys::HSQUIRRELVM,
}

impl SQVM {
	pub fn new(stack_size: i64) -> Self {
		let ptr = unsafe { squirrel_sys::sq_open(stack_size) };

		// A boxed `VmContext` holds the (optional) output handlers and is reached
		// from the C trampolines via the VM's foreign pointer. Ownership lives with
		// this `SQVM`; `Drop` frees it after closing the VM.
		let context = Box::into_raw(VmContext::boxed());
		unsafe {
			squirrel_sys::sq_setforeignptr(ptr, context as squirrel_sys::SQUserPointer);
			squirrel_sys::sq_setprintfunc(ptr, Some(print_trampoline), Some(error_trampoline));
			squirrel_sys::sq_setcompilererrorhandler(ptr, Some(compile_error_trampoline));

			// Register the parts of the Squirrel standard library scripts use: the
			// string library (`format`/`split`/`strip`/`regexp`/…) and the math
			// library (`abs`/`sin`/`sqrt`/… at the root). Both install into the root
			// table, so this must happen after `sq_open`. The system/io/blob libs are
			// deliberately left out (no filesystem/process access for game scripts).
			squirrel_sys::sq_pushroottable(ptr);
			squirrel_sys::sqstd_register_stringlib(ptr);
			squirrel_sys::sqstd_register_mathlib(ptr);
			squirrel_sys::sq_pop(ptr, 1);
		}

		SQVM { ptr }
	}

	fn context_ref(&self) -> &VmContext {
		// SAFETY: as `context`; shared access only.
		unsafe { &*(squirrel_sys::sq_getforeignptr(self.ptr) as *const VmContext) }
	}

	fn context(&mut self) -> &mut VmContext {
		// SAFETY: `new` installed a non-null `VmContext` as the foreign pointer and
		// only `Drop` clears it; `&mut self` ensures exclusive access.
		unsafe { &mut *(squirrel_sys::sq_getforeignptr(self.ptr) as *mut VmContext) }
	}

	/// Route the VM's `print` output to `handler` instead of stdout.
	pub fn set_print_handler(&mut self, handler: impl Fn(&str) + 'static) {
		self.context().set_print(Box::new(handler));
	}

	/// Route the VM's `error` output to `handler` instead of stderr.
	pub fn set_error_handler(&mut self, handler: impl Fn(&str) + 'static) {
		self.context().set_error(Box::new(handler));
	}

	/// Route compile errors to `handler` instead of stderr.
	pub fn set_compile_error_handler(&mut self, handler: impl Fn(&CompileError) + 'static) {
		self.context().set_compile_error(Box::new(handler));
	}

	/// Install the Squirrel stdlib error handlers: an unhandled runtime error then
	/// prints a full call stack (function + source:line + locals) via the print
	/// handler before the call returns. Invaluable for locating a script error;
	/// call after [`set_print_handler`](Self::set_print_handler). Compile errors keep
	/// going to [`set_compile_error_handler`](Self::set_compile_error_handler) (the
	/// stdlib would take those over too), so [`compile_capturing`](Self::compile_capturing)
	/// still works.
	pub fn install_standard_error_handlers(&mut self) {
		unsafe {
			squirrel_sys::sqstd_seterrorhandlers(self.as_ptr());
			squirrel_sys::sq_setcompilererrorhandler(self.as_ptr(), Some(compile_error_trampoline));
		}
	}

	/// Hand every uncaught runtime error to `handler`, as data (the thrown value and the
	/// call stack with each frame's locals), before the failing call returns. Replaces
	/// [`install_standard_error_handlers`](Self::install_standard_error_handlers)'
	/// runtime part (the last call wins); compile errors keep going to
	/// [`set_compile_error_handler`](Self::set_compile_error_handler).
	pub fn set_runtime_error_handler(&mut self, handler: impl Fn(&RuntimeError) + 'static) {
		self.context().set_runtime_error(Box::new(handler));
		let context = self.context() as *mut VmContext;
		unsafe {
			// The context rides along as the closure's free variable: threads have no
			// foreign pointer of their own.
			squirrel_sys::sq_pushuserpointer(self.as_ptr(), context as squirrel_sys::SQUserPointer);
			squirrel_sys::sq_newclosure(self.as_ptr(), Some(runtime_error::runtime_error_trampoline), 1);
			squirrel_sys::sq_seterrorhandler(self.as_ptr());
		}
	}

	/// Run the cycle collector: free the objects that only reference cycles keep alive
	/// (reference counting frees everything else as soon as it's unreachable). Objects
	/// held by a [`SqStrongRef`](crate::SqStrongRef), the root table, the registry or a
	/// running call's stack stay. Returns how many objects it freed.
	pub fn collect_garbage(&self) -> i64 {
		unsafe { squirrel_sys::sq_collectgarbage(self.ptr) }
	}

	/// The objects that only reference cycles keep alive, as an array, without freeing
	/// them (`None` if there are none): what [`collect_garbage`](Self::collect_garbage)
	/// would free. For finding what leaks.
	pub fn unreachable(&self) -> Option<crate::SqStrongRef> {
		unsafe {
			if squirrel_sys::sq_resurrectunreachable(self.ptr) < 0 {
				return None;
			}
			let found = squirrel_sys::sq_gettype(self.ptr, -1) == squirrel_sys::tagSQObjectType_OT_ARRAY;
			let array = found.then(|| crate::SqStrongRef::from_stack(self, -1));
			squirrel_sys::sq_pop(self.ptr, 1);
			array
		}
	}

	/// The raw VM handle. Crate-internal: the safe wrappers in the sibling
	/// modules go through this rather than touching the private `ptr` field.
	pub(crate) fn as_ptr(&self) -> squirrel_sys::HSQUIRRELVM {
		self.ptr
	}

	/// Borrow a raw handle without taking ownership, so `Drop` will not close it.
	/// Used inside the native-call trampoline, where Squirrel owns the VM.
	pub(crate) unsafe fn from_raw(ptr: squirrel_sys::HSQUIRRELVM) -> std::mem::ManuallyDrop<Self> {
		std::mem::ManuallyDrop::new(SQVM { ptr })
	}
}

impl Drop for SQVM {
	fn drop(&mut self) {
		unsafe {
			// Take back the boxed `VmContext` we installed as the foreign pointer,
			// then close the VM and drop it. Closing first means no callback can
			// fire against a freed context.
			let context = squirrel_sys::sq_getforeignptr(self.ptr) as *mut VmContext;
			squirrel_sys::sq_close(self.ptr);
			if !context.is_null() {
				drop(Box::from_raw(context));
			}
		}
	}
}
