//! String and bool parameters read as the original engine's Sqrat bindings read them:
//! any value, converted, never an error (from the original's disassembly).
//!
//! - [`Stringified`]: Sqrat's `Var<std::string>` (`0x4a33e0`) calls `sq_tostring` on the
//!   argument (`0x586d10`), so any value is its `tostring()` text (`null` is `"null"`, an
//!   array `"(array : 0x…)"`, an instance with `_tostring` what that returns), then
//!   copies it as a C string: up to its first zero byte.
//! - [`Truthy`]: Sqrat's `Var<bool>` (`0x586df0`) is `sq_tobool`: null, 0, 0.0 and false
//!   are false, every other value true.

use super::SqArg;
use crate::{SQ_OK, SQVM};

/// Any argument as the original's string parameters read it (see [the module docs](self)).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stringified(pub Vec<u8>);

impl SqArg for Stringified {
	type Item<'a> = Stringified;
	fn push_typemask(mask: &mut String) {
		mask.push('.');
	}
	fn from_sq<'a>(vm: &'a SQVM, idx: i64) -> Result<Stringified, String> {
		// `sq_tostring` pushes the text (a negative index is read before the push).
		if unsafe { squirrel_sys::sq_tostring(vm.as_ptr(), idx) } != SQ_OK {
			unsafe { squirrel_sys::sq_reseterror(vm.as_ptr()) };
			return Ok(Stringified::default());
		}
		let mut text = vm.get_bytes_ref(-1).map(<[u8]>::to_vec).unwrap_or_default();
		unsafe { squirrel_sys::sq_pop(vm.as_ptr(), 1) };
		if let Some(end) = text.iter().position(|&b| b == 0) {
			text.truncate(end);
		}
		Ok(Stringified(text))
	}
}

/// Any argument as the original's bool parameters read it (see [the module docs](self)).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Truthy(pub bool);

impl SqArg for Truthy {
	type Item<'a> = Truthy;
	fn push_typemask(mask: &mut String) {
		mask.push('.');
	}
	fn from_sq<'a>(vm: &'a SQVM, idx: i64) -> Result<Truthy, String> {
		let mut b: squirrel_sys::SQBool = 0;
		unsafe { squirrel_sys::sq_tobool(vm.as_ptr(), idx, &mut b) };
		Ok(Truthy(b != 0))
	}
}

#[cfg(test)]
mod tests {
	use std::cell::RefCell;
	use std::rc::Rc;

	use super::{Stringified, Truthy};
	use crate::{SQVM, SqTable};

	#[test]
	fn any_value_reads_as_its_text_and_its_truth() {
		let mut vm = SQVM::new(1024);
		let seen: Rc<RefCell<Vec<(String, bool)>>> = Rc::default();
		let sink = seen.clone();
		let see = vm.make_closure(move |s: Stringified, b: Truthy| {
			sink.borrow_mut().push((String::from_utf8(s.0).unwrap(), b.0));
		});
		SqTable::root(&vm).set("see", see);
		let nul = vm.make_closure(|| crate::SqBytes(b"a\0b".to_vec()));
		SqTable::root(&vm).set("nul", nul);
		vm.run_script(
			"foreach (v in [null, true, false, 0, 5, 0.0, 2.5, \"\", \"abc\", nul()]) see(v, v);
			see(class { function _tostring() { return \"custom\"; } }(), 0);
			see([1], []);",
			"test",
		)
		.unwrap();
		let got = seen.take();
		let expect = [
			("null", false),
			("true", true),
			("false", false),
			("0", false),
			("5", true),
			("0", false),
			("2.5", true),
			("", true),
			("abc", true),
			("a", true),
			("custom", false),
		];
		for (i, (text, truth)) in expect.iter().enumerate() {
			assert_eq!(got[i], (text.to_string(), *truth), "value {i}");
		}
		assert!(got[11].0.starts_with("(array : 0x") && got[11].1, "{:?}", got[11]);
	}
}
