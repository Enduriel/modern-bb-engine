//! BB logging: `logInfo` / `logDebug` / `logWarning` / `logError` / `error`, plus
//! the no-op console-grouping helpers (`printStack` / `logBeginGroup` /
//! `logEndGroup`). Output is routed to a host-provided handler — where logs go
//! (file, console, in-game UI) is the host's concern, mirroring the VM's
//! `set_print_handler`.
//!
//! Scripts call these as `this.logInfo(...)`, which resolves to the root-table
//! entry via Squirrel's bytecode root fallback.

use std::rc::Rc;

use squirrel::{SQVM, SqTable};

/// Severity passed to a log handler.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
	Debug,
	Info,
	Warning,
	Error,
}

/// The default handler used by [`install_natives`](crate::install_natives):
/// warnings/errors to stderr, info/debug to stdout, each level-prefixed.
pub(crate) fn default_handler(level: LogLevel, message: &str) {
	match level {
		LogLevel::Warning | LogLevel::Error => eprintln!("[{level:?}] {message}"),
		LogLevel::Debug | LogLevel::Info => println!("[{level:?}] {message}"),
	}
}

/// Register the logging functions into the root table, routing each to `handler`.
///
/// Re-registering (e.g. after [`install_natives`](crate::install_natives)) simply
/// overwrites the previous handler — the last call wins.
pub fn install_logging(vm: &mut SQVM, handler: impl Fn(LogLevel, &str) + 'static) {
	let handler: Rc<dyn Fn(LogLevel, &str)> = Rc::new(handler);

	let at = |level: LogLevel, handler: &Rc<dyn Fn(LogLevel, &str)>, vm: &mut SQVM| {
		let handler = handler.clone();
		vm.make_closure(move |message: &str| handler(level, message))
	};

	let info = at(LogLevel::Info, &handler, vm);
	let debug = at(LogLevel::Debug, &handler, vm);
	let warning = at(LogLevel::Warning, &handler, vm);
	// `logError` (SQ source) and `error` (Script source) are both ERROR-level; the
	// source distinction is a UI concern we do not model.
	let log_error = at(LogLevel::Error, &handler, vm);
	let error = at(LogLevel::Error, &handler, vm);

	// The console-grouping + stack-dump helpers are UI features with no SQ-side
	// effect; register them as no-ops so scripts can call them freely.
	let print_stack = super::stubs::closure(vm, "printStack");
	let begin_group = vm.make_closure(|_label: &str| {});
	let end_group = super::stubs::closure(vm, "logEndGroup");

	let mut root = SqTable::root(vm);
	root.set("logInfo", info);
	root.set("logDebug", debug);
	root.set("logWarning", warning);
	root.set("logError", log_error);
	root.set("error", error);
	root.set("printStack", print_stack);
	root.set("logBeginGroup", begin_group);
	root.set("logEndGroup", end_group);
}
