//! The script call stack as data, read from inside a native: for a native's own
//! diagnostics (which script asked for what), without exposing the stack itself.

use super::SQVM;
use super::runtime_error::{Frame, frames_from};

impl SQVM {
	/// The calls on the stack right now, innermost first (a native reading it is the
	/// first frame), without their locals.
	pub fn call_stack(&self) -> Vec<Frame> {
		frames_from(self, 0, false)
	}
}

#[cfg(test)]
mod tests {
	use std::cell::RefCell;
	use std::rc::Rc;

	use crate::{SQVM, SqTable};

	#[test]
	fn a_native_sees_the_scripts_that_called_it() {
		let mut vm = SQVM::new(1024);
		let seen: Rc<RefCell<Vec<(Option<String>, Option<String>, i64)>>> = Rc::default();
		let sink = seen.clone();
		let native = vm.make_closure(move |vm: &SQVM| {
			*sink.borrow_mut() = vm.call_stack().into_iter().map(|f| (f.function, f.source, f.line)).collect();
		});
		SqTable::root(&vm).set("probe", native);
		vm.run_script("function inner() {\n probe();\n}\nfunction outer() { inner(); }\nouter();", "calls.nut").unwrap();
		let frames = seen.take();
		assert_eq!(frames[0].1.as_deref(), Some("NATIVE"));
		assert_eq!(frames[1], (Some("inner".to_string()), Some("calls.nut".to_string()), 2));
		assert_eq!(frames[2], (Some("outer".to_string()), Some("calls.nut".to_string()), 4));
	}
}
