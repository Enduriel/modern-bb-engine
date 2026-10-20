//! Which VM this thread is closing, so strong refs dropped by `sq_close` itself
//! release nothing.
//!
//! # The teardown contract
//!
//! An [`SqStrongRef`](crate::SqStrongRef) holds an `sq_addref` and releases it
//! against its VM on `Drop`. There are two kinds of owners:
//!
//! * **Rust outside the VM** (host services, schedulers, caches). These must drop
//!   their refs while the VM is still open: `squirrel_bb::Engine` clears its
//!   services before it closes the VM. A ref dropped after the VM is gone releases
//!   against freed memory.
//! * **The VM's own objects**: the Rust state of a native closure (its captures)
//!   or of a native class (its member slots, instance payloads). The VM drops these
//!   when it frees the object, so they can drop *during* `sq_close`. Objects kept
//!   alive only by a reference cycle are freed last, after `sq_close` has freed the
//!   root VM and emptied the reference table that `sq_release` works on. Releasing
//!   then reads the freed VM.
//!
//! The second kind cannot be dropped any earlier, because the VM owns it. So while
//! a VM closes, its refs release nothing. Nothing leaks: `sq_close` frees every
//! object, whatever the reference counts say, and the reference table has already
//! dropped the references these refs stood for.
//!
//! The VM and its strong refs are not `Send`, so a ref is dropped on the thread that
//! closes its VM, and a thread-local is enough.

use std::cell::Cell;

thread_local! {
	static CLOSING: Cell<squirrel_sys::HSQUIRRELVM> = const { Cell::new(std::ptr::null_mut()) };
}

/// Marks `vm` as closing on this thread until dropped.
pub(crate) struct Closing {
	previous: squirrel_sys::HSQUIRRELVM,
}

impl Closing {
	pub(crate) fn begin(vm: squirrel_sys::HSQUIRRELVM) -> Self {
		Closing { previous: CLOSING.replace(vm) }
	}
}

impl Drop for Closing {
	fn drop(&mut self) {
		CLOSING.set(self.previous);
	}
}

/// Whether `vm` is being closed on this thread, so a ref against it must not be
/// released.
#[inline]
pub(crate) fn is_closing(vm: squirrel_sys::HSQUIRRELVM) -> bool {
	CLOSING.get() == vm
}
