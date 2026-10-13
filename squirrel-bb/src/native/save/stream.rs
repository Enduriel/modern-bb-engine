//! The save streams scripts write into and read from (`_out.writeU32(...)`,
//! `_in.readString()`, `_in.getMetaData().getVersion()`), plus the byte buffers behind
//! them that the engine's own save code shares.
//!
//! Encoding (our format): little-endian fixed-width integers, `f32` as IEEE bits, a
//! bool as one byte, a string as a `u32` byte length then UTF-8. Values are truncated to
//! the written width, as a C++ cast would.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use squirrel::{ClassBuilder, Number, SQVM, SqClass, SqStrongRef};

/// Bytes being written.
#[derive(Default)]
pub(crate) struct SaveWriter {
	pub buf: Vec<u8>,
}

impl SaveWriter {
	pub fn u8(&mut self, v: u8) {
		self.buf.push(v);
	}
	pub fn u16(&mut self, v: u16) {
		self.buf.extend_from_slice(&v.to_le_bytes());
	}
	pub fn u32(&mut self, v: u32) {
		self.buf.extend_from_slice(&v.to_le_bytes());
	}
	pub fn i64(&mut self, v: i64) {
		self.buf.extend_from_slice(&v.to_le_bytes());
	}
	pub fn f32(&mut self, v: f32) {
		self.buf.extend_from_slice(&v.to_le_bytes());
	}
	pub fn f64(&mut self, v: f64) {
		self.buf.extend_from_slice(&v.to_le_bytes());
	}
	pub fn bool(&mut self, v: bool) {
		self.u8(u8::from(v));
	}
	pub fn str(&mut self, v: &str) {
		self.u32(v.len() as u32);
		self.buf.extend_from_slice(v.as_bytes());
	}
}

/// Bytes being read.
pub(crate) struct SaveReader {
	buf: Vec<u8>,
	pos: usize,
}

impl SaveReader {
	pub fn new(buf: Vec<u8>) -> Self {
		SaveReader { buf, pos: 0 }
	}

	fn take<const N: usize>(&mut self) -> Result<[u8; N], String> {
		let end = self.pos + N;
		let bytes = self.buf.get(self.pos..end).ok_or("save data ends early")?;
		self.pos = end;
		Ok(bytes.try_into().expect("slice of length N"))
	}

	pub fn u8(&mut self) -> Result<u8, String> {
		Ok(self.take::<1>()?[0])
	}
	pub fn u16(&mut self) -> Result<u16, String> {
		Ok(u16::from_le_bytes(self.take()?))
	}
	pub fn u32(&mut self) -> Result<u32, String> {
		Ok(u32::from_le_bytes(self.take()?))
	}
	pub fn i64(&mut self) -> Result<i64, String> {
		Ok(i64::from_le_bytes(self.take()?))
	}
	pub fn f32(&mut self) -> Result<f32, String> {
		Ok(f32::from_le_bytes(self.take()?))
	}
	pub fn f64(&mut self) -> Result<f64, String> {
		Ok(f64::from_le_bytes(self.take()?))
	}
	pub fn bool(&mut self) -> Result<bool, String> {
		Ok(self.u8()? != 0)
	}
	pub fn str(&mut self) -> Result<String, String> {
		let len = self.u32()? as usize;
		let end = self.pos + len;
		let bytes = self.buf.get(self.pos..end).ok_or("save data ends early")?;
		self.pos = end;
		String::from_utf8(bytes.to_vec()).map_err(|_| "save string is not UTF-8".to_string())
	}
}

/// A save's metadata: what the load menu lists, plus what `onBeforeSerialize` records.
#[derive(Clone, Default)]
pub(crate) struct Meta {
	pub version: i64,
	/// The label shown in menus (`getName`).
	pub name: String,
	/// The storage name the save is written under (`getFileName`).
	pub file_name: String,
	/// Seconds since the Unix epoch.
	pub created: i64,
	pub ints: BTreeMap<String, i64>,
	pub strings: BTreeMap<String, String>,
}

impl Meta {
	pub fn write(&self, w: &mut SaveWriter) {
		w.i64(self.version);
		w.str(&self.name);
		w.str(&self.file_name);
		w.i64(self.created);
		w.u32(self.ints.len() as u32);
		for (k, v) in &self.ints {
			w.str(k);
			w.i64(*v);
		}
		w.u32(self.strings.len() as u32);
		for (k, v) in &self.strings {
			w.str(k);
			w.str(v);
		}
	}

	pub fn read(r: &mut SaveReader) -> Result<Self, String> {
		let mut meta = Meta {
			version: r.i64()?,
			name: r.str()?,
			file_name: r.str()?,
			created: r.i64()?,
			..Meta::default()
		};
		for _ in 0..r.u32()? {
			let k = r.str()?;
			meta.ints.insert(k, r.i64()?);
		}
		for _ in 0..r.u32()? {
			let k = r.str()?;
			meta.strings.insert(k, r.str()?);
		}
		Ok(meta)
	}
}

pub(crate) type SharedMeta = Rc<RefCell<Meta>>;

/// The script classes: an output stream, an input stream and a metadata object.
pub(crate) struct StreamClasses {
	out: SqClass<OutStream>,
	input: SqClass<InStream>,
	meta: SqClass<SharedMeta>,
}

#[derive(Clone)]
struct OutStream {
	writer: Rc<RefCell<SaveWriter>>,
	meta: SharedMeta,
	meta_class: SqClass<SharedMeta>,
}

#[derive(Clone)]
struct InStream {
	reader: Rc<RefCell<SaveReader>>,
	meta: SharedMeta,
	meta_class: SqClass<SharedMeta>,
}

/// A script integer argument as the original casts it (floats truncate).
fn int(n: Number) -> i64 {
	n.0 as i64
}

impl StreamClasses {
	pub fn build(vm: &mut SQVM) -> Self {
		let meta = ClassBuilder::<SharedMeta>::new("SaveMetaData")
			.method("getVersion", |m: &mut SharedMeta| m.borrow().version)
			.method("setVersion", |m: &mut SharedMeta, v: Number| m.borrow_mut().version = int(v))
			.method("getName", |m: &mut SharedMeta| m.borrow().name.clone())
			.method("getFileName", |m: &mut SharedMeta| m.borrow().file_name.clone())
			.method("getCreationDate", |m: &mut SharedMeta| m.borrow().created)
			.method("getInt", |m: &mut SharedMeta, k: String| m.borrow().ints.get(&k).copied().unwrap_or(0))
			.method("setInt", |m: &mut SharedMeta, k: String, v: Number| {
				m.borrow_mut().ints.insert(k, int(v));
			})
			.method("getString", |m: &mut SharedMeta, k: String| m.borrow().strings.get(&k).cloned().unwrap_or_default())
			.method("setString", |m: &mut SharedMeta, k: String, v: String| {
				m.borrow_mut().strings.insert(k, v);
			})
			.build(vm);

		let out = ClassBuilder::<OutStream>::new("SaveOutputStream")
			.method("writeU8", |s: &mut OutStream, v: Number| s.writer.borrow_mut().u8(int(v) as u8))
			.method("writeI8", |s: &mut OutStream, v: Number| s.writer.borrow_mut().u8(int(v) as i8 as u8))
			.method("writeU16", |s: &mut OutStream, v: Number| s.writer.borrow_mut().u16(int(v) as u16))
			.method("writeI16", |s: &mut OutStream, v: Number| s.writer.borrow_mut().u16(int(v) as i16 as u16))
			.method("writeU32", |s: &mut OutStream, v: Number| s.writer.borrow_mut().u32(int(v) as u32))
			.method("writeI32", |s: &mut OutStream, v: Number| s.writer.borrow_mut().u32(int(v) as i32 as u32))
			.method("writeF32", |s: &mut OutStream, v: Number| s.writer.borrow_mut().f32(v.0 as f32))
			.method("writeBool", |s: &mut OutStream, v: bool| s.writer.borrow_mut().bool(v))
			.method("writeString", |s: &mut OutStream, v: String| s.writer.borrow_mut().str(&v))
			.method("getMetaData", |s: &mut OutStream, vm: &SQVM| -> SqStrongRef {
				s.meta_class.instantiate(vm, s.meta.clone())
			})
			.build(vm);

		let input = ClassBuilder::<InStream>::new("SaveInputStream")
			.method("readU8", |s: &mut InStream| -> Result<i64, String> { Ok(s.reader.borrow_mut().u8()?.into()) })
			.method("readI8", |s: &mut InStream| -> Result<i64, String> {
				Ok((s.reader.borrow_mut().u8()? as i8).into())
			})
			.method("readU16", |s: &mut InStream| -> Result<i64, String> { Ok(s.reader.borrow_mut().u16()?.into()) })
			.method("readI16", |s: &mut InStream| -> Result<i64, String> {
				Ok((s.reader.borrow_mut().u16()? as i16).into())
			})
			// The original's script integers are 32-bit, so a U32 at or above 2^31 reads
			// back negative; scripts rely on it (a hash written with writeI32 is read
			// with readU32 and looked up again).
			.method("readU32", |s: &mut InStream| -> Result<i64, String> {
				Ok((s.reader.borrow_mut().u32()? as i32).into())
			})
			.method("readI32", |s: &mut InStream| -> Result<i64, String> {
				Ok((s.reader.borrow_mut().u32()? as i32).into())
			})
			.method("readF32", |s: &mut InStream| -> Result<f64, String> { Ok(s.reader.borrow_mut().f32()?.into()) })
			.method("readBool", |s: &mut InStream| -> Result<bool, String> { s.reader.borrow_mut().bool() })
			.method("readString", |s: &mut InStream| -> Result<String, String> { s.reader.borrow_mut().str() })
			.method("getMetaData", |s: &mut InStream, vm: &SQVM| -> SqStrongRef {
				s.meta_class.instantiate(vm, s.meta.clone())
			})
			.build(vm);

		StreamClasses { out, input, meta }
	}

	/// A script output stream over `writer`, with `meta`.
	pub fn out_stream(&self, vm: &SQVM, writer: Rc<RefCell<SaveWriter>>, meta: SharedMeta) -> SqStrongRef {
		let stream = OutStream {
			writer,
			meta,
			meta_class: self.meta.clone(),
		};
		self.out.instantiate(vm, stream)
	}

	/// A script input stream over `reader`, with `meta`.
	pub fn in_stream(&self, vm: &SQVM, reader: Rc<RefCell<SaveReader>>, meta: SharedMeta) -> SqStrongRef {
		let stream = InStream {
			reader,
			meta,
			meta_class: self.meta.clone(),
		};
		self.input.instantiate(vm, stream)
	}

	/// A script metadata object (for `PersistenceManager.queryStorages`).
	pub fn meta_object(&self, vm: &SQVM, meta: SharedMeta) -> SqStrongRef {
		self.meta.instantiate(vm, meta)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn round_trips_every_width() {
		let mut w = SaveWriter::default();
		w.u8(250);
		w.u8(-3_i8 as u8);
		w.u16(65000);
		w.u32(4_000_000_000);
		w.u32(-7_i32 as u32);
		w.f32(1.5);
		w.bool(true);
		w.str("Grimald");
		w.i64(-123_456_789_012);
		let mut r = SaveReader::new(w.buf);
		assert_eq!(r.u8().unwrap(), 250);
		assert_eq!(r.u8().unwrap() as i8, -3);
		assert_eq!(r.u16().unwrap(), 65000);
		assert_eq!(r.u32().unwrap(), 4_000_000_000);
		assert_eq!(r.u32().unwrap() as i32, -7);
		assert_eq!(r.f32().unwrap(), 1.5);
		assert!(r.bool().unwrap());
		assert_eq!(r.str().unwrap(), "Grimald");
		assert_eq!(r.i64().unwrap(), -123_456_789_012);
		assert!(r.u8().is_err());
	}

	#[test]
	fn meta_round_trips() {
		let mut meta = Meta {
			version: 72,
			name: "My Company".into(),
			file_name: "my_company".into(),
			created: 1_760_000_000,
			..Meta::default()
		};
		meta.ints.insert("days".into(), 12);
		meta.strings.insert("banner".into(), "banner_04".into());
		let mut w = SaveWriter::default();
		meta.write(&mut w);
		let back = Meta::read(&mut SaveReader::new(w.buf)).unwrap();
		assert_eq!(back.version, 72);
		assert_eq!(back.name, "My Company");
		assert_eq!(back.ints["days"], 12);
		assert_eq!(back.strings["banner"], "banner_04");
	}
}
