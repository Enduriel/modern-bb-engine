//! BB logging: `logInfo` / `logDebug` / `logWarning` / `logError` / `error`, plus
//! the no-op console-grouping helpers (`printStack` / `logBeginGroup` /
//! `logEndGroup`). Each line goes to log.html when one is open ([`crate::html_log`],
//! tagged as the original tags it) and to a host-provided handler (the terminal by
//! default), mirroring the VM's `set_print_handler`.
//!
//! Scripts call these as `this.logInfo(...)`, which resolves to the root-table
//! entry via Squirrel's bytecode root fallback.

use std::rc::Rc;

use squirrel::{SQVM, SqObjectType, SqRef, SqTable};

use crate::html_log::{self, Class};

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

impl From<LogLevel> for Class {
	fn from(level: LogLevel) -> Class {
		match level {
			LogLevel::Debug => Class::Debug,
			LogLevel::Info => Class::Info,
			LogLevel::Warning => Class::Warning,
			LogLevel::Error => Class::Error,
		}
	}
}

/// What the original logs for a value given to `logInfo` and kin (probed 2026-10-20): a
/// string as is (its bytes, whatever they are), a number or bool as `tostring` writes
/// it, `null` as `(null : 0x00000000)`, anything else as `(<type> : 0x<address>)`.
/// Addresses are the original's own; ours writes zeros there.
fn message_bytes(value: &SqRef) -> Vec<u8> {
	let text = match value {
		SqRef::String(s) => return s.clone().into_bytes(),
		SqRef::Bytes(b) => return b.clone(),
		SqRef::Integer(i) => i.to_string(),
		SqRef::Float(f) => crate::html_log::value::float_g6(*f),
		SqRef::Bool(b) => b.to_string(),
		SqRef::Null => "(null : 0x00000000)".into(),
		SqRef::Array(_) => "(array : 0x00000000)".into(),
		SqRef::Table(_) => "(table : 0x00000000)".into(),
		SqRef::Other(kind) => format!("({} : 0x00000000)", type_name(*kind)),
		_ => "(object : 0x00000000)".into(),
	};
	text.into_bytes()
}

/// Squirrel's name for a type, as `typeof` gives it.
fn type_name(kind: SqObjectType) -> &'static str {
	match kind {
		SqObjectType::Closure | SqObjectType::NativeClosure => "function",
		SqObjectType::Class => "class",
		SqObjectType::Instance => "instance",
		SqObjectType::Generator => "generator",
		SqObjectType::UserData => "userdata",
		SqObjectType::UserPointer => "userpointer",
		SqObjectType::Thread => "thread",
		SqObjectType::WeakRef => "weakref",
		SqObjectType::FuncProto => "funcproto",
		_ => "object",
	}
}

/// Register the logging functions into the root table, routing each to `handler`.
///
/// Re-registering (e.g. after [`install_natives`](crate::install_natives)) simply
/// overwrites the previous handler — the last call wins.
pub fn install_logging(vm: &mut SQVM, handler: impl Fn(LogLevel, &str) + 'static) {
	let handler: Rc<dyn Fn(LogLevel, &str)> = Rc::new(handler);

	// Each goes to log.html (when one is open) under the original's tag, and to `handler`.
	let at = |level: LogLevel, tag: &'static str, handler: &Rc<dyn Fn(LogLevel, &str)>, vm: &mut SQVM| {
		let handler = handler.clone();
		vm.make_closure(move |message: SqRef| {
			let bytes = message_bytes(&message);
			html_log::line(level.into(), tag, &bytes);
			handler(level, &String::from_utf8_lossy(&bytes));
		})
	};

	let info = at(LogLevel::Info, "SQ", &handler, vm);
	let debug = at(LogLevel::Debug, "SQ", &handler, vm);
	let warning = at(LogLevel::Warning, "SQ", &handler, vm);
	// `logError` and `error` are both ERROR-level; the original tags `error`'s lines
	// `Script` (probed).
	let log_error = at(LogLevel::Error, "SQ", &handler, vm);
	let error = at(LogLevel::Error, "Script", &handler, vm);

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

#[cfg(test)]
mod tests {
	use std::cell::RefCell;
	use std::rc::Rc;

	use super::*;

	#[test]
	fn any_value_is_logged_as_the_original_logs_it() {
		let mut vm = SQVM::new(1024);
		let seen: Rc<RefCell<Vec<(LogLevel, String)>>> = Rc::default();
		let sink = seen.clone();
		install_logging(&mut vm, move |level, text| sink.borrow_mut().push((level, text.to_string())));
		vm.run_script(
			"logInfo(5); logInfo(1.5); logInfo(null); logDebug(true); logWarning({}); logError(\"a\\nb\"); error(0xE9.tochar());",
			"test",
		)
		.unwrap();
		let seen = seen.take();
		let texts: Vec<_> = seen.iter().map(|(_, t)| t.as_str()).collect();
		// The original's text for each (probe 2026-10-20), but the table's address.
		assert_eq!(texts, ["5", "1.5", "(null : 0x00000000)", "true", "(table : 0x00000000)", "a\nb", "\u{FFFD}"]);
		assert_eq!(seen[3].0, LogLevel::Debug);
		assert_eq!(seen[6].0, LogLevel::Error);
		// Still one argument, as the original's.
		assert!(vm.run_script("logInfo();", "test").is_err());
	}
}
