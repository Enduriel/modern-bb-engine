//! A little-endian cursor over a byte slice, for the binary asset formats.

/// Reads little-endian values from a byte slice, failing (not panicking) at the end.
pub(crate) struct Reader<'a> {
	bytes: &'a [u8],
	pos: usize,
}

impl<'a> Reader<'a> {
	pub(crate) fn new(bytes: &'a [u8]) -> Self {
		Reader { bytes, pos: 0 }
	}

	/// The next `len` bytes; `what` names the field for the error message.
	pub(crate) fn take(&mut self, len: usize, what: &str) -> Result<&'a [u8], String> {
		let end = self
			.pos
			.checked_add(len)
			.filter(|&end| end <= self.bytes.len());
		let Some(end) = end else {
			return Err(format!("truncated at byte {} reading {what}", self.pos));
		};
		let slice = &self.bytes[self.pos..end];
		self.pos = end;
		Ok(slice)
	}

	pub(crate) fn array<const N: usize>(&mut self, what: &str) -> Result<[u8; N], String> {
		let slice = self.take(N, what)?;
		Ok(slice.try_into().expect("take returned N bytes"))
	}

	pub(crate) fn u8(&mut self, what: &str) -> Result<u8, String> {
		Ok(self.array::<1>(what)?[0])
	}

	pub(crate) fn i8(&mut self, what: &str) -> Result<i8, String> {
		Ok(self.u8(what)? as i8)
	}

	pub(crate) fn u16(&mut self, what: &str) -> Result<u16, String> {
		Ok(u16::from_le_bytes(self.array(what)?))
	}

	pub(crate) fn i16(&mut self, what: &str) -> Result<i16, String> {
		Ok(i16::from_le_bytes(self.array(what)?))
	}

	pub(crate) fn u32(&mut self, what: &str) -> Result<u32, String> {
		Ok(u32::from_le_bytes(self.array(what)?))
	}

	pub(crate) fn i32(&mut self, what: &str) -> Result<i32, String> {
		Ok(i32::from_le_bytes(self.array(what)?))
	}

	pub(crate) fn u64(&mut self, what: &str) -> Result<u64, String> {
		Ok(u64::from_le_bytes(self.array(what)?))
	}

	pub(crate) fn f32(&mut self, what: &str) -> Result<f32, String> {
		Ok(f32::from_le_bytes(self.array(what)?))
	}

	/// A string stored as a `u16` byte length and then the bytes (no terminator).
	pub(crate) fn string(&mut self, what: &str) -> Result<String, String> {
		let len = self.u16(what)?;
		let bytes = self.take(usize::from(len), what)?;
		Ok(String::from_utf8_lossy(bytes).into_owned())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn reads_little_endian_values_in_order() {
		let bytes = [
			0x01, 0x34, 0x12, 0x00, 0x00, 0x80, 0x3f, 0x02, 0x00, b'h', b'i',
		];
		let mut r = Reader::new(&bytes);
		assert_eq!(r.u8("a"), Ok(1));
		assert_eq!(r.u16("b"), Ok(0x1234));
		assert_eq!(r.f32("c"), Ok(1.0));
		assert_eq!(r.string("d"), Ok("hi".to_string()));
		assert!(r.u8("e").is_err());
	}

	#[test]
	fn reports_truncation_with_the_field_name() {
		let mut r = Reader::new(&[1, 2, 3]);
		assert_eq!(r.u16("first"), Ok(0x0201));
		assert_eq!(
			r.u32("second"),
			Err("truncated at byte 2 reading second".to_string())
		);
	}

	#[test]
	fn a_string_longer_than_the_input_is_an_error() {
		let mut r = Reader::new(&[5, 0, b'a']);
		assert!(r.string("name").is_err());
	}
}
