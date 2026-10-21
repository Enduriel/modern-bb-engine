//! Where the VM reaches the profiler: its native debug hook (script calls and returns)
//! and the native trampoline ([`native_enter`]/[`native_exit`]). The recorder of the
//! profiling under way on this thread is reached through a thread-local pointer, set
//! only while [`Attached`] lives.

use std::cell::Cell;
use std::ffi::{CStr, c_char};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use super::FunctionKind;
use super::recorder::{FunctionInfo, Key, Recorder};
use crate::SQVM;

/// How many threads are profiling: the native trampoline's only cost when none is.
static ATTACHED: AtomicUsize = AtomicUsize::new(0);

thread_local! {
	static CURRENT: Cell<*mut Recorder> = const { Cell::new(std::ptr::null_mut()) };
}

/// Real time in nanoseconds since the first reading.
fn now() -> u64 {
	static ORIGIN: OnceLock<Instant> = OnceLock::new();
	ORIGIN.get_or_init(Instant::now).elapsed().as_nanos() as u64
}

/// Run `f` on this thread's recorder, if one is attached, with the event's virtual
/// time; the time `f` takes is the profiler's overhead.
#[inline]
fn event<R>(f: impl FnOnce(&mut Recorder, u64) -> R) -> Option<R> {
	let recorder = CURRENT.with(Cell::get);
	if recorder.is_null() {
		return None;
	}
	let start = now();
	// SAFETY: `Attached` set the pointer from a `&mut Recorder` it borrows for its own
	// life and clears it before that ends; nothing else touches the recorder meanwhile
	// (the hook never calls back into the VM).
	let recorder = unsafe { &mut *recorder };
	let virtual_now = recorder.virtual_time(start);
	let result = f(recorder, virtual_now);
	recorder.add_overhead(now() - start);
	Some(result)
}

fn text(ptr: *const c_char) -> Option<String> {
	(!ptr.is_null()).then(|| unsafe { CStr::from_ptr(ptr) }.to_string_lossy().into_owned())
}

unsafe extern "C" fn debug_hook(
	_vm: squirrel_sys::HSQUIRRELVM,
	kind: squirrel_sys::SQInteger,
	source: *const c_char,
	line: squirrel_sys::SQInteger,
	name: *const c_char,
) {
	match kind as u8 {
		b'c' => {
			let key = Key { a: source as usize, b: name as usize, line };
			event(|r, now| {
				r.enter(now, key, Some((key.a, key.b)), || FunctionInfo {
					kind: FunctionKind::Script,
					name: text(name),
					source: text(source),
					line,
				})
			});
		}
		b'r' => {
			event(|r, now| r.script_return(now, (source as usize, name as usize)));
		}
		_ => {}
	}
}

/// A native function labelled `label()` is called (each label a function's own string,
/// or a type's name, shared by the functions of that type): the open-call depth to hand
/// [`native_exit`] when it returns, or `None` when not profiling. The label is only
/// asked for while profiling.
#[inline]
pub(crate) fn native_enter<'a>(label: impl FnOnce() -> &'a str) -> Option<usize> {
	if ATTACHED.load(Ordering::Relaxed) == 0 {
		return None;
	}
	let label = label();
	let key = Key { a: label.as_ptr() as usize, b: label.len(), line: -1 };
	event(|r, now| {
		r.enter(now, key, None, || FunctionInfo {
			kind: FunctionKind::Native,
			name: Some(label.to_string()),
			source: None,
			line: 0,
		})
	})
}

/// The native call [`native_enter`] opened at `depth` returns.
#[inline]
pub(crate) fn native_exit(depth: usize) {
	event(|r, now| r.native_return(now, depth));
}

/// The profiler attached to a VM, for as long as this lives.
pub(super) struct Attached<'a> {
	vm: squirrel_sys::HSQUIRRELVM,
	_recorder: std::marker::PhantomData<&'a mut Recorder>,
}

impl<'a> Attached<'a> {
	/// Attach `recorder` to `vm`, unless this thread already profiles.
	pub fn new(vm: &SQVM, recorder: &'a mut Recorder) -> Option<Self> {
		if !CURRENT.with(Cell::get).is_null() {
			return None;
		}
		recorder.begin(now());
		CURRENT.with(|c| c.set(recorder));
		ATTACHED.fetch_add(1, Ordering::Relaxed);
		unsafe { squirrel_sys::sq_setnativedebughook(vm.as_ptr(), Some(debug_hook)) };
		Some(Attached { vm: vm.as_ptr(), _recorder: std::marker::PhantomData })
	}
}

impl Drop for Attached<'_> {
	fn drop(&mut self) {
		unsafe { squirrel_sys::sq_setnativedebughook(self.vm, None) };
		ATTACHED.fetch_sub(1, Ordering::Relaxed);
		let recorder = CURRENT.with(|c| c.replace(std::ptr::null_mut()));
		// SAFETY: as in `event`; the borrow `new` took is still live.
		let recorder = unsafe { &mut *recorder };
		let real = now();
		recorder.end(real);
	}
}
