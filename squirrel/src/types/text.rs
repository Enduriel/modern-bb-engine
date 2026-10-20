//! [`SqText`]: any value as `tostring` writes it.

use super::SqValue;
use crate::{SQ_OK, SQVM, SqObjectType};

/// A value as the script's `tostring(value)` gives it (its bytes, whatever they are):
/// a `_tostring` metamethod's result for an instance that has one, and for an object
/// without one `(<type> : 0x<16 hex digits>)`, the same text for the same live object.
/// `None` for `null` (and for a `_tostring` that fails), so a caller can say what the
/// original says there. As a native's argument it takes any value (type mask `.`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SqText(pub Option<Vec<u8>>);

impl SqText {
	/// The text of the value at `idx`.
	pub(crate) fn of(vm: &SQVM, idx: i64) -> SqText {
		if vm.get_type(idx) == SqObjectType::Null {
			return SqText(None);
		}
		// `sq_tostring` pushes the text (a negative index is read before the push).
		if unsafe { squirrel_sys::sq_tostring(vm.as_ptr(), idx) } != SQ_OK {
			unsafe { squirrel_sys::sq_reseterror(vm.as_ptr()) };
			return SqText(None);
		}
		let text = vm.get_bytes_ref(-1).map(<[u8]>::to_vec);
		unsafe { squirrel_sys::sq_pop(vm.as_ptr(), 1) };
		SqText(text)
	}
}

impl SqValue for SqText {
	type Item<'a> = SqText;

	fn push_value(self, vm: &SQVM) {
		match self.0 {
			Some(text) => vm.push_bytes(&text),
			None => vm.push_null(),
		}
	}

	fn read<'a>(vm: &'a SQVM, idx: i64) -> Option<SqText> {
		Some(SqText::of(vm, idx))
	}
}

#[cfg(test)]
mod tests {
	use std::cell::RefCell;
	use std::rc::Rc;

	use super::SqText;
	use crate::{SQVM, SqTable};

	/// What `see(x)` received, for each call `source` makes.
	fn texts(source: &str) -> Vec<SqText> {
		let mut vm = SQVM::new(1024);
		let seen: Rc<RefCell<Vec<SqText>>> = Rc::default();
		let sink = seen.clone();
		let see = vm.make_closure(move |t: SqText| sink.borrow_mut().push(t));
		SqTable::root(&vm).set("see", see);
		vm.run_script(source, "test").unwrap();
		seen.take()
	}

	fn text(t: &SqText) -> String {
		String::from_utf8(t.0.clone().unwrap()).unwrap()
	}

	#[test]
	fn values_read_as_tostring_writes_them() {
		let got = texts("foreach (v in [5, true, \"s\", null, {}, [], function() {}]) see(v);");
		assert_eq!(text(&got[0]), "5");
		assert_eq!(text(&got[1]), "true");
		assert_eq!(text(&got[2]), "s");
		assert_eq!(got[3], SqText(None));
		// The original engine's shape (MSVC's %p: no prefix of its own, uppercase, fixed
		// width), with the whole 64-bit address.
		for (t, kind) in [(&got[4], "table"), (&got[5], "array"), (&got[6], "function")] {
			let t = text(t);
			let hex = t.strip_prefix(&format!("({kind} : 0x")).and_then(|r| r.strip_suffix(')')).unwrap_or_else(|| panic!("{t}"));
			assert_eq!(hex.len(), 16, "{t}");
			assert!(hex.chars().all(|c| c.is_ascii_digit() || ('A'..='F').contains(&c)), "{t}");
		}
	}

	#[test]
	fn the_same_object_gives_the_same_text() {
		let got = texts("local t = {}; see(t); see(t); see({}); see(t.tostring());");
		assert_eq!(got[0], got[1]);
		assert_eq!(got[0], got[3]);
		assert_ne!(got[0], got[2]);
	}
}
