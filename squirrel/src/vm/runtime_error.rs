//! An uncaught runtime error, as data: the thrown value and the call stack with each
//! frame's locals ([`RuntimeError`]), handed to the handler a consumer installs with
//! [`SQVM::set_runtime_error_handler`](super::SQVM::set_runtime_error_handler). The VM
//! calls its error handler (a native closure here) before the failing call returns;
//! this module reads the stack there, so no stack access leaks out.

use std::ffi::{CStr, c_char};

use super::print::VmContext;
use crate::SqObjectType;

/// A value as an error report shows it: scalars and strings by value, the rest by
/// type only.
#[derive(Clone, Debug, PartialEq)]
pub enum ErrorValue {
	Null,
	Integer(i64),
	Float(f64),
	Bool(bool),
	/// A string's bytes, as the script has them (not necessarily UTF-8).
	String(Vec<u8>),
	Table,
	Array,
	UserData,
	Closure,
	NativeClosure,
	Generator,
	UserPointer,
	Thread,
	FuncProto,
	Class,
	Instance,
	WeakRef,
	Outer,
}

/// A named value of a frame: a local variable, parameter or free variable.
#[derive(Clone, Debug, PartialEq)]
pub struct Local {
	pub name: String,
	pub value: ErrorValue,
}

/// One call on the stack, innermost first.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
	/// The function's name, `None` for an anonymous script function. Native functions
	/// without a name are `"unknown"`, as the VM reports them.
	pub function: Option<String>,
	/// The source the function was compiled from (`"NATIVE"` for a native function).
	pub source: Option<String>,
	/// The line being run (-1 for a native function).
	pub line: i64,
	/// Its locals as the VM lists them (the latest declared first, `this` last); none
	/// for a native function.
	pub locals: Vec<Local>,
}

/// An error no script caught.
#[derive(Clone, Debug, PartialEq)]
pub struct RuntimeError {
	/// What was thrown (the VM's own errors are strings).
	pub error: ErrorValue,
	pub frames: Vec<Frame>,
}

/// Read the value at `idx` of `vm`'s stack as an [`ErrorValue`].
fn value_at(vm: &super::SQVM, idx: i64) -> ErrorValue {
	match vm.get_type(idx) {
		SqObjectType::Null => ErrorValue::Null,
		SqObjectType::Integer => ErrorValue::Integer(vm.get_integer(idx).unwrap_or(0)),
		SqObjectType::Float => ErrorValue::Float(vm.get_float(idx).unwrap_or(0.0)),
		SqObjectType::Bool => ErrorValue::Bool(vm.get_bool(idx).unwrap_or(false)),
		SqObjectType::String => ErrorValue::String(vm.get_bytes_ref(idx).unwrap_or_default().to_vec()),
		SqObjectType::Table => ErrorValue::Table,
		SqObjectType::Array => ErrorValue::Array,
		SqObjectType::UserData => ErrorValue::UserData,
		SqObjectType::Closure => ErrorValue::Closure,
		SqObjectType::NativeClosure => ErrorValue::NativeClosure,
		SqObjectType::Generator => ErrorValue::Generator,
		SqObjectType::UserPointer => ErrorValue::UserPointer,
		SqObjectType::Thread => ErrorValue::Thread,
		SqObjectType::FuncProto => ErrorValue::FuncProto,
		SqObjectType::Class => ErrorValue::Class,
		SqObjectType::Instance => ErrorValue::Instance,
		SqObjectType::WeakRef => ErrorValue::WeakRef,
		SqObjectType::Outer => ErrorValue::Outer,
	}
}

fn text(ptr: *const c_char) -> Option<String> {
	(!ptr.is_null()).then(|| unsafe { CStr::from_ptr(ptr) }.to_string_lossy().into_owned())
}

/// The locals of the call `level` frames down.
fn locals(vm: &super::SQVM, level: i64) -> Vec<Local> {
	let mut out = Vec::new();
	let mut idx = 0;
	loop {
		let name = unsafe { squirrel_sys::sq_getlocal(vm.as_ptr(), level as _, idx) };
		let Some(name) = text(name) else { break };
		out.push(Local { name, value: value_at(vm, -1) });
		unsafe { squirrel_sys::sq_pop(vm.as_ptr(), 1) };
		idx += 1;
	}
	out
}

/// The call stack below the error handler (level 0), innermost first.
fn frames(vm: &super::SQVM) -> Vec<Frame> {
	let mut out = Vec::new();
	let mut level = 1;
	loop {
		let mut si: squirrel_sys::SQStackInfos = unsafe { std::mem::zeroed() };
		if unsafe { squirrel_sys::sq_stackinfos(vm.as_ptr(), level, &mut si) } < 0 {
			break;
		}
		out.push(Frame {
			function: text(si.funcname),
			source: text(si.source),
			line: si.line,
			locals: locals(vm, level),
		});
		level += 1;
	}
	out
}

/// The VM's error handler: called with (this, error) and, as its free variable, the
/// VM's [`VmContext`] (a thread's own foreign pointer is unset, so it travels here).
pub(crate) unsafe extern "C" fn runtime_error_trampoline(ptr: squirrel_sys::HSQUIRRELVM) -> squirrel_sys::SQInteger {
	let vm = unsafe { super::SQVM::from_raw(ptr) };
	let top = vm.stack_top();
	let mut context: squirrel_sys::SQUserPointer = std::ptr::null_mut();
	if unsafe { squirrel_sys::sq_getuserpointer(ptr, top, &mut context) } < 0 || context.is_null() {
		return 0;
	}
	let context = unsafe { &*(context as *const VmContext) };
	let Some(handler) = context.runtime_error() else { return 0 };
	let error = if top >= 3 { value_at(&vm, 2) } else { ErrorValue::Null };
	handler(&RuntimeError { error, frames: frames(&vm) });
	0
}

#[cfg(test)]
mod tests {
	use std::cell::RefCell;
	use std::rc::Rc;

	use super::*;
	use crate::SQVM;

	fn run(source: &str) -> Vec<RuntimeError> {
		let mut vm = SQVM::new(1024);
		let seen: Rc<RefCell<Vec<RuntimeError>>> = Rc::default();
		let sink = seen.clone();
		vm.set_runtime_error_handler(move |e| sink.borrow_mut().push(e.clone()));
		let _ = vm.run_script(source, "test.nut");
		seen.take()
	}

	#[test]
	fn reports_the_stack_with_locals() {
		let errors = run(
			"function inner(_a) { local f = 1.5, s = \"x\", t = {}; throw \"boom\"; }\n\
			 function outer() { local n = null; inner(3); }\n\
			 (function() { outer(); })();",
		);
		assert_eq!(errors.len(), 1);
		let e = &errors[0];
		assert_eq!(e.error, ErrorValue::String(b"boom".to_vec()));
		let inner = &e.frames[0];
		assert_eq!(inner.function.as_deref(), Some("inner"));
		assert_eq!(inner.source.as_deref(), Some("test.nut"));
		assert_eq!(inner.line, 1);
		let locals: Vec<_> = inner.locals.iter().map(|l| (l.name.as_str(), l.value.clone())).collect();
		assert_eq!(
			locals,
			[
				("t", ErrorValue::Table),
				("s", ErrorValue::String(b"x".to_vec())),
				("f", ErrorValue::Float(1.5)),
				("_a", ErrorValue::Integer(3)),
				("this", ErrorValue::Table),
			]
		);
		assert_eq!(e.frames[1].function.as_deref(), Some("outer"));
		assert_eq!(e.frames[1].line, 2);
		// An anonymous function has no name; the script's own is `main`.
		assert_eq!(e.frames[2].function, None);
		assert_eq!(e.frames[3].function.as_deref(), Some("main"));
	}

	#[test]
	fn passes_on_what_was_thrown() {
		let errors = run("throw 5;");
		assert_eq!(errors[0].error, ErrorValue::Integer(5));
	}

	#[test]
	fn reports_errors_in_threads() {
		let errors = run("local t = ::newthread(function() { local z = null; z.foo(); }); t.call();");
		assert!(!errors.is_empty());
		assert_eq!(errors[0].error, ErrorValue::String(b"the index 'foo' does not exist".to_vec()));
	}
}
