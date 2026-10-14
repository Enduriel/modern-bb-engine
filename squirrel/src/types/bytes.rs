//! [`SqBytes`]: a Squirrel string as its raw bytes.

use super::SqValue;
use crate::SQVM;

/// A Squirrel string's bytes, whether or not they're UTF-8. Squirrel strings are bytes,
/// and scripts build any (`0xE9.tochar()` is Windows-1252's é), so a value the engine
/// keeps for the scripts unchanged (e.g. a label's text, which the original draws one
/// glyph per byte) reads as this rather than as a `String`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SqBytes(pub Vec<u8>);

impl SqValue for SqBytes {
	type Item<'a> = SqBytes;

	fn push_value(self, vm: &SQVM) {
		vm.push_bytes(&self.0);
	}

	fn read<'a>(vm: &'a SQVM, idx: i64) -> Option<SqBytes> {
		vm.get_bytes_ref(idx).map(|b| SqBytes(b.to_vec()))
	}
}

#[cfg(test)]
mod tests {
	use super::SqBytes;
	use crate::{SQVM, SqArray};

	#[test]
	fn any_string_round_trips() {
		let vm = SQVM::new(1024);
		let mut arr = SqArray::new(&vm);
		arr.push(SqBytes(vec![0x41, 0xE9, 0x80]));
		arr.push("é".to_string());
		assert_eq!(arr.get::<SqBytes>(0), Some(SqBytes(vec![0x41, 0xE9, 0x80])));
		assert_eq!(arr.get::<SqBytes>(1), Some(SqBytes(vec![0xC3, 0xA9])));
	}
}
