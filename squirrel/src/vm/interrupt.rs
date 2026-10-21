//! Stopping a script from another thread: an [`Interrupter`] (from
//! [`SQVM::interrupter`]) asks a VM to stop, and from then on every native function
//! registered through this crate that a script of that VM calls raises an error instead
//! of running. The error unwinds the script like any other, so the runtime error handler
//! sees the call stack where it was, e.g. a script stuck in a loop that calls natives.
//! A loop that calls no native function (or only the standard library's) isn't stopped.
//!
//! Made for watchdogs (a headless run's timeout). Costs one relaxed atomic load per
//! native call while no VM is asked to stop.

use std::sync::atomic::{AtomicUsize, Ordering};

use super::SQVM;

/// The root VM asked to stop (its address), or 0. One at a time: a process runs one VM
/// that a watchdog guards.
static TARGET: AtomicUsize = AtomicUsize::new(0);

/// The message of the error a native call raises while its VM is asked to stop.
pub const INTERRUPTED: &str = "interrupted: the host asked the scripts to stop";

/// Asks one VM to stop its scripts, from any thread. Copy it into the watchdog.
#[derive(Clone, Copy, Debug)]
pub struct Interrupter {
	root: usize,
}

impl Interrupter {
	/// From now on, every native call of the VM's scripts raises [`INTERRUPTED`].
	pub fn request(&self) {
		TARGET.store(self.root, Ordering::Relaxed);
	}

	/// Let the VM's natives run again.
	pub fn withdraw(&self) {
		let _ = TARGET.compare_exchange(self.root, 0, Ordering::Relaxed, Ordering::Relaxed);
	}
}

fn root_of(vm: squirrel_sys::HSQUIRRELVM) -> usize {
	unsafe { squirrel_sys::sq_getrootvm(vm) as usize }
}

impl SQVM {
	/// What stops this VM's scripts from another thread (see the module docs).
	pub fn interrupter(&self) -> Interrupter {
		Interrupter { root: root_of(self.as_ptr()) }
	}
}

/// Whether `vm`'s root VM is asked to stop; checked as a native function is called.
#[inline]
pub(crate) fn requested(vm: squirrel_sys::HSQUIRRELVM) -> bool {
	let target = TARGET.load(Ordering::Relaxed);
	target != 0 && target == root_of(vm)
}

/// `root` closes: a stale request mustn't stop a later VM at the same address.
pub(super) fn forget(root: squirrel_sys::HSQUIRRELVM) {
	let _ = TARGET.compare_exchange(root as usize, 0, Ordering::Relaxed, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn an_interrupted_vm_raises_at_its_next_native_call() {
		let mut vm = SQVM::new(1024);
		let calls = std::rc::Rc::new(std::cell::Cell::new(0));
		{
			let calls = calls.clone();
			vm.register("tick", move || calls.set(calls.get() + 1));
		}
		let errors = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
		{
			let errors = errors.clone();
			vm.set_runtime_error_handler(move |e| {
				let frames: Vec<String> = e.frames.iter().filter_map(|f| f.function.clone()).collect();
				let message = match &e.error {
					crate::ErrorValue::String(b) => String::from_utf8_lossy(b).into_owned(),
					other => format!("{other:?}"),
				};
				errors.borrow_mut().push((message, frames));
			});
		}
		vm.run_script("function busy() { tick(); tick(); }", "t").unwrap();
		vm.run_script("busy();", "t").unwrap();
		assert_eq!(calls.get(), 2);

		let interrupter = vm.interrupter();
		interrupter.request();
		// A loop with no exit of its own ends at its next native call.
		assert!(vm.run_script("function spin() { while (true) tick(); } spin();", "t").is_err());
		assert_eq!(calls.get(), 2);
		let errors = errors.borrow();
		let (message, frames) = errors.last().unwrap();
		assert_eq!(message, INTERRUPTED);
		assert!(frames.iter().any(|f| f == "spin"), "{frames:?}");

		interrupter.withdraw();
		vm.run_script("busy();", "t").unwrap();
		assert_eq!(calls.get(), 4);
	}

	#[test]
	fn another_vm_keeps_running() {
		let mut a = SQVM::new(1024);
		let mut b = SQVM::new(1024);
		a.register("tick", || {});
		b.register("tick", || {});
		a.interrupter().request();
		assert!(a.run_script("tick();", "t").is_err());
		assert!(b.run_script("tick();", "t").is_ok());
		a.interrupter().withdraw();
		assert!(a.run_script("tick();", "t").is_ok());
	}
}
