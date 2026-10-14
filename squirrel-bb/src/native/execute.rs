//! `execute(code)`: compile and run a string of Squirrel, as the original's root
//! native does (probed 2026-10-14). It runs with the root table as `this` and always
//! returns null. A compile error is logged as `Failed to execute string "<code>".
//! Reason: "<message>" line:<l> column:<c>.` and nothing runs; a runtime error goes to
//! the VM's error handler like any script error, and the caller carries on.

use squirrel::{SQVM, SqStrongRef, SqTable};

/// Register `execute` into the root table.
pub(crate) fn install(vm: &mut SQVM) {
	let execute = vm.make_closure(|vm: &SQVM, code: String| {
		match vm.compile_capturing(&code, "execute") {
			Ok(closure) => {
				// The error handler has reported a runtime error already.
				let _ = closure.call(vm, ());
			}
			Err(e) => {
				let message = format!(
					"Failed to execute string \"{code}\". Reason: \"{}\" line:{} column:{}.",
					e.message, e.line, e.column
				);
				if let Some(log) = SqTable::root(vm).get::<SqStrongRef>("logError") {
					let _ = log.call(vm, (message,));
				}
			}
		}
	});
	SqTable::root(vm).set("execute", execute);
}
