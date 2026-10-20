//! Per-VM output context: the print / error / compile-error handlers a consumer
//! can install, dispatched from C trampolines via the VM's foreign pointer.
//!
//! Consumers set high-level `Fn(&str)` handlers (see
//! [`SQVM::set_print_handler`](super::SQVM::set_print_handler)); this module does
//! the printf formatting and dispatch, so no `squirrel-sys` types leak out. With
//! no handler installed, output goes to stdout/stderr as before.

use std::ffi::{CStr, VaList, c_char, c_int, c_void};

unsafe extern "C" {
	// `scvprintf` maps to these in non-Unicode builds.
	unsafe fn vsnprintf(
		buf: *mut c_char,
		size: usize,
		format: *const c_char,
		ap: VaList<'_>,
	) -> c_int;
	unsafe fn vfprintf(stream: *mut c_void, format: *const c_char, ap: VaList<'_>) -> c_int;

	// On glibc, `stdout`/`stderr` are real exported `FILE *` symbols.
	unsafe static mut stdout: *mut c_void;
	unsafe static mut stderr: *mut c_void;
}

/// A compile error reported by the VM, in a Rust-friendly form.
pub struct CompileError {
	pub message: String,
	pub source: String,
	pub line: i64,
	pub column: i64,
}

/// Rust-side per-VM state reachable from C callbacks via the foreign pointer.
/// Owned by the [`SQVM`](super::SQVM) (boxed, freed on `Drop`).
pub(crate) struct VmContext {
	print: Option<Box<dyn Fn(&str)>>,
	error: Option<Box<dyn Fn(&str)>>,
	compile_error: Option<Box<dyn Fn(&CompileError)>>,
	runtime_error: Option<Box<dyn Fn(&super::RuntimeError)>>,
	/// While set, compile errors are kept here instead of going to the handler
	/// ([`SQVM::compile_capturing`](super::SQVM::compile_capturing)).
	captured: std::cell::RefCell<Option<Option<CompileError>>>,
}

impl VmContext {
	pub(crate) fn boxed() -> Box<Self> {
		Box::new(VmContext {
			print: None,
			error: None,
			compile_error: None,
			runtime_error: None,
			captured: std::cell::RefCell::new(None),
		})
	}

	pub(crate) fn set_print(&mut self, handler: Box<dyn Fn(&str)>) {
		self.print = Some(handler);
	}
	pub(crate) fn set_error(&mut self, handler: Box<dyn Fn(&str)>) {
		self.error = Some(handler);
	}
	pub(crate) fn set_compile_error(&mut self, handler: Box<dyn Fn(&CompileError)>) {
		self.compile_error = Some(handler);
	}

	pub(crate) fn set_runtime_error(&mut self, handler: Box<dyn Fn(&super::RuntimeError)>) {
		self.runtime_error = Some(handler);
	}
	pub(crate) fn runtime_error(&self) -> Option<&dyn Fn(&super::RuntimeError)> {
		self.runtime_error.as_deref()
	}

	/// Start keeping compile errors instead of reporting them.
	pub(crate) fn begin_capture(&self) {
		*self.captured.borrow_mut() = Some(None);
	}

	/// Stop keeping them; the error kept since [`begin_capture`](Self::begin_capture).
	pub(crate) fn end_capture(&self) -> Option<CompileError> {
		self.captured.borrow_mut().take().flatten()
	}
}

/// Borrow the VM's `VmContext` (set in `SQVM::new`), or `None` if absent.
unsafe fn context<'a>(vm: squirrel_sys::HSQUIRRELVM) -> Option<&'a VmContext> {
	let ptr = unsafe { squirrel_sys::sq_getforeignptr(vm) } as *const VmContext;
	if ptr.is_null() {
		None
	} else {
		Some(unsafe { &*ptr })
	}
}

/// Format a printf-style `(format, args)` and hand the text to `f`.
///
/// A small stack buffer covers short messages with no heap allocation. If the
/// message is larger, `vsnprintf` reports its exact length, so we allocate
/// precisely that and re-format (from a `va_copy`d cursor). No truncation, and no
/// wasteful fixed buffer — large debug prints work and small ones stay cheap.
unsafe fn with_formatted(format: *const c_char, args: VaList<'_>, f: impl FnOnce(&str)) {
	const INLINE: usize = 256;
	let mut stack = [0u8; INLINE];
	// First pass into the stack buffer, on a `va_copy` (`args` survives for a
	// possible second pass). `vsnprintf` returns the full length it needs, even
	// when that exceeds the buffer.
	let needed = unsafe {
		vsnprintf(
			stack.as_mut_ptr() as *mut c_char,
			INLINE,
			format,
			args.clone(),
		)
	};
	if needed < 0 {
		return; // formatting error — emit nothing
	}
	let needed = needed as usize;
	if needed < INLINE {
		let text = String::from_utf8_lossy(&stack[..needed]);
		f(&text);
	} else {
		let mut heap = vec![0u8; needed + 1];
		unsafe { vsnprintf(heap.as_mut_ptr() as *mut c_char, heap.len(), format, args) };
		let text = String::from_utf8_lossy(&heap[..needed]);
		f(&text);
	}
}

pub(crate) unsafe extern "C" fn print_trampoline(
	vm: squirrel_sys::HSQUIRRELVM,
	format: *const c_char,
	args: ...
) {
	match unsafe { context(vm) }.and_then(|c| c.print.as_ref()) {
		Some(handler) => unsafe { with_formatted(format, args, |text| handler(text)) },
		None => {
			unsafe { vfprintf(stdout, format, args) };
		}
	}
}

pub(crate) unsafe extern "C" fn error_trampoline(
	vm: squirrel_sys::HSQUIRRELVM,
	format: *const c_char,
	args: ...
) {
	match unsafe { context(vm) }.and_then(|c| c.error.as_ref()) {
		Some(handler) => unsafe { with_formatted(format, args, |text| handler(text)) },
		None => {
			unsafe { vfprintf(stderr, format, args) };
		}
	}
}

pub(crate) unsafe extern "C" fn compile_error_trampoline(
	vm: squirrel_sys::HSQUIRRELVM,
	description: *const c_char,
	source: *const c_char,
	line: i64,
	column: i64,
) {
	let message = unsafe { CStr::from_ptr(description) }
		.to_str()
		.unwrap_or("invalid UTF-8 in error")
		.to_owned();
	let source = unsafe { CStr::from_ptr(source) }
		.to_str()
		.unwrap_or("invalid UTF-8 in source")
		.to_owned();

	let ctx = unsafe { context(vm) };
	if let Some(slot) = ctx.map(|c| &c.captured) {
		if let Some(captured) = slot.borrow_mut().as_mut() {
			*captured = Some(CompileError { message, source, line, column });
			return;
		}
	}
	match ctx.and_then(|c| c.compile_error.as_ref()) {
		Some(handler) => handler(&CompileError {
			message,
			source,
			line,
			column,
		}),
		None => eprintln!("Compile error in '{source}', line {line}, column {column}: {message}"),
	}
}
