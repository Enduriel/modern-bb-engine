//! Transcode a Battle Brothers `.cnut` (Squirrel 3.0.4, 32-bit int+float,
//! XXTEA-encrypted) into our VM's bytecode-stream format (Squirrel 3.1, 64-bit),
//! so it can be loaded with `SQVM::load_closure`.
//!
//! The two `sq_writeclosure` formats are field-for-field identical (same opcodes,
//! same `SQClosure::Save` / `SQFunctionProto::Save` sequence, same `PART`/`HEAD`/
//! `TAIL` markers) — only the *widths* differ: `SQInteger`/`SQFloat`/
//! `SQUnsignedInteger` are 4 bytes in the game and 8 in ours, and the header's
//! size markers must be rewritten (1/4/4 → 1/8/8).
//!
//! ## The one structural change: hoisting inline floats
//!
//! `SQInstruction` is 8 bytes in both builds, so the instruction block is *almost*
//! copied verbatim — with a single exception. The game's 32-bit-float compiler emits
//! float literals as an inline `OP_LOADFLOAT` whose immediate is the 4-byte `_arg1`
//! field (`TARGET = *((SQFloat*)&_arg1)`). With `SQFloat=f64` that read is 8 bytes
//! wide and overlaps the `op` byte at offset 4 → it can never load a real f64. A
//! 64-bit Squirrel never produces this instruction in the first place: its compiler's
//! `EmitLoadConstFloat` takes the `else` branch (`sizeof(SQFloat) != sizeof(SQInt32)`)
//! and stores the double in the numeric-constant pool via `GetNumericConstant`, then
//! emits `OP_LOAD <idx>`. So we reproduce exactly that decision on already-compiled
//! bytecode: every inline `OP_LOADFLOAT(target, f32)` becomes `OP_LOAD(target, idx)`
//! pointing at an `OT_FLOAT` f64 we append to that proto's literal table (deduped by
//! bit pattern, like `GetNumericConstant`). This means each proto must be parsed into
//! buffers and re-emitted, since the grown literal count + table precede the
//! instructions in the stream. (`OP_LOADINT` needs no such fix: our VM does
//! `(SQInteger)((SQInt32)arg1)`, sign-extending the 4-byte immediate to i64.)
//!
//! The XXTEA decryption is applied per *write chunk*: every `SafeWrite`/`WriteTag`
//! is one chunk, and only chunks of ≥ 8 bytes are encrypted — so the small scalar
//! writes (tags, counts, single ints/floats, the `bool`) are plaintext, and only
//! the bulk blocks (long strings, the line-info / default-param / instruction
//! arrays) are encrypted. We walk the same sequence, decrypting each block read.

use std::collections::HashMap;

use crate::crypt;

// Stream marker constants (read little-endian; values from `sqobject.h`).
const TAG_FAFA: u16 = 0xFAFA; // sq_writeclosure stream tag
const HEAD: u32 = ('S' as u32) << 24 | ('Q' as u32) << 16 | ('I' as u32) << 8 | ('R' as u32);
const PART: u32 = ('P' as u32) << 24 | ('A' as u32) << 16 | ('R' as u32) << 8 | ('T' as u32);
const TAIL: u32 = ('T' as u32) << 24 | ('A' as u32) << 16 | ('I' as u32) << 8 | ('L' as u32);

// Raw object-type tags (`type & _RT_MASK`); identical across 3.0.4 and 3.1.
const RT_MASK: u32 = 0x00FF_FFFF;
const RT_NULL: u32 = 0x01;
const RT_INTEGER: u32 = 0x02;
const RT_FLOAT: u32 = 0x04;
const RT_BOOL: u32 = 0x08;
const RT_STRING: u32 = 0x10;

// Full object-type tag for a float literal:
// `OT_FLOAT = _RT_FLOAT | SQOBJECT_NUMERIC | SQOBJECT_CANBEFALSE` (squirrel.h). Written
// when we hoist an inline `OP_LOADFLOAT` immediate into the proto's literal table.
const OT_FLOAT: u32 = 0x0500_0004;

// Instruction opcodes touched by the float hoist (`sqopcodes.h`). The instruction is
// `[ _arg1: i32 (bytes 0..4) ][ op: u8 (byte 4) ][ _arg0, _arg2, _arg3 (bytes 5..8) ]`.
const OP_LOAD: u8 = 0x01; // TARGET = _literals[arg1]
const OP_LOADFLOAT: u8 = 0x03; // TARGET = *((SQFloat*)&arg1) — inline, broken under f64

type TResult<T> = Result<T, String>;

/// Transcode an encrypted 3.0.4 `.cnut` into our VM's closure-stream bytes.
pub fn transcode_cnut(cnut: &[u8]) -> TResult<Vec<u8>> {
	let mut r = Reader::new(cnut);
	let mut w = Writer::new();

	if r.u16()? != TAG_FAFA {
		return Err("not a compiled closure (missing 0xFAFA tag)".into());
	}
	w.u16(TAG_FAFA);

	// Closure header: HEAD, then sizeof(char/int/float). The game's are 1/4/4; we
	// emit ours (1/8/8) so our sq_readclosure's size checks pass.
	expect_u32(&mut r, &mut w, HEAD, "stream head")?;
	let char_size = r.u32()?;
	let int_size = r.u32()?;
	let float_size = r.u32()?;
	if char_size != 1 || int_size != 4 || float_size != 4 {
		return Err(format!(
			"unexpected source sizes (char={char_size}, int={int_size}, float={float_size}); \
			 expected a 32-bit Squirrel 3.0.4 build"
		));
	}
	w.u32(1); // sizeof(SQChar)
	w.u32(8); // sizeof(SQInteger)
	w.u32(8); // sizeof(SQFloat)

	transcode_proto(&mut r, &mut w)?;

	expect_u32(&mut r, &mut w, TAIL, "stream tail")?;
	Ok(w.into_bytes())
}

fn transcode_proto(r: &mut Reader, w: &mut Writer) -> TResult<()> {
	expect_u32(r, w, PART, "proto start")?;
	transcode_object(r, w)?; // _sourcename
	transcode_object(r, w)?; // _name

	// Counts are captured, not emitted yet: hoisting inline floats grows `nliterals`,
	// and both the count and the literal table precede the instructions in the stream.
	expect_marker(r, PART, "proto counts")?;
	let nliterals = read_count(r)?;
	let nparameters = read_count(r)?;
	let noutervalues = read_count(r)?;
	let nlocalvarinfos = read_count(r)?;
	let nlineinfos = read_count(r)?;
	let ndefaultparams = read_count(r)?;
	let ninstructions = read_count(r)?;
	let nfunctions = read_count(r)?;

	// Each section is transcoded into its own buffer so we can re-emit in stream order
	// after the instruction pass tells us how many floats to append to the literals.
	expect_marker(r, PART, "literals")?;
	let mut literals = Writer::new();
	for _ in 0..nliterals {
		transcode_object(r, &mut literals)?;
	}

	expect_marker(r, PART, "parameters")?;
	let mut parameters = Writer::new();
	for _ in 0..nparameters {
		transcode_object(r, &mut parameters)?;
	}

	expect_marker(r, PART, "outer values")?;
	let mut outervalues = Writer::new();
	for _ in 0..noutervalues {
		widen_unsigned(r, &mut outervalues)?; // _type (SQUnsignedInteger)
		transcode_object(r, &mut outervalues)?; // _src
		transcode_object(r, &mut outervalues)?; // _name
	}

	expect_marker(r, PART, "local var infos")?;
	let mut localvarinfos = Writer::new();
	for _ in 0..nlocalvarinfos {
		transcode_object(r, &mut localvarinfos)?; // _name
		widen_unsigned(r, &mut localvarinfos)?; // _pos
		widen_unsigned(r, &mut localvarinfos)?; // _start_op
		widen_unsigned(r, &mut localvarinfos)?; // _end_op
	}

	// Each of the next three is ONE write chunk (so ONE XXTEA block), then split
	// into elements and re-widened.
	expect_marker(r, PART, "line infos")?;
	let mut lineinfos = Writer::new();
	let li_block = r.block(nlineinfos * 8)?; // SQLineInfo = 2 * int32
	for li in li_block.chunks_exact(8) {
		lineinfos.i64(le_i32(&li[0..4]) as i64); // _line
		lineinfos.i64(le_i32(&li[4..8]) as i64); // _op
	}

	expect_marker(r, PART, "default params")?;
	let mut defaultparams = Writer::new();
	let dp_block = r.block(ndefaultparams * 4)?;
	for dp in dp_block.chunks_exact(4) {
		defaultparams.i64(le_i32(dp) as i64);
	}

	// Instruction pass: rewrite each inline `OP_LOADFLOAT(target, f32)` into an
	// `OP_LOAD(target, idx)` against a float we append to the literal table (deduped
	// by raw bit pattern, matching the compiler's `GetNumericConstant`).
	expect_marker(r, PART, "instructions")?;
	let raw = r.block(ninstructions * 8)?; // SQInstruction = 8 bytes in both
	let mut instructions = Writer::new();
	let mut hoisted: Vec<f32> = Vec::new();
	let mut hoisted_index: HashMap<u32, i64> = HashMap::new();
	for ins in raw.chunks_exact(8) {
		let mut b = [ins[0], ins[1], ins[2], ins[3], ins[4], ins[5], ins[6], ins[7]];
		if b[4] == OP_LOADFLOAT {
			let bits = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
			let idx = *hoisted_index.entry(bits).or_insert_with(|| {
				let i = (nliterals + hoisted.len()) as i64;
				hoisted.push(f32::from_bits(bits));
				i
			});
			b[4] = OP_LOAD; // op:   _OP_LOADFLOAT -> _OP_LOAD
			b[0..4].copy_from_slice(&(idx as i32).to_le_bytes()); // _arg1: f32 bits -> literal index
		}
		instructions.bytes(&b);
	}

	expect_marker(r, PART, "sub-functions")?;
	let mut subfunctions = Writer::new();
	for _ in 0..nfunctions {
		transcode_proto(r, &mut subfunctions)?;
	}

	let stacksize = r.i32()?; // _stacksize (SQInteger)
	let bgenerator = r.u8()?; // _bgenerator (C++ bool, 1 byte in both)
	let varparams = r.i32()?; // _varparams (SQInteger)

	// Re-emit in stream order, with the literal table grown by the hoisted floats.
	w.u32(PART); // proto counts
	w.i64((nliterals + hoisted.len()) as i64);
	w.i64(nparameters as i64);
	w.i64(noutervalues as i64);
	w.i64(nlocalvarinfos as i64);
	w.i64(nlineinfos as i64);
	w.i64(ndefaultparams as i64);
	w.i64(ninstructions as i64);
	w.i64(nfunctions as i64);

	w.u32(PART); // literals
	w.bytes(literals.as_bytes());
	for f in &hoisted {
		w.u32(OT_FLOAT);
		w.f64(*f as f64); // value conversion (f32 -> f64), as GetNumericConstant would store
	}

	w.u32(PART); // parameters
	w.bytes(parameters.as_bytes());
	w.u32(PART); // outer values
	w.bytes(outervalues.as_bytes());
	w.u32(PART); // local var infos
	w.bytes(localvarinfos.as_bytes());
	w.u32(PART); // line infos
	w.bytes(lineinfos.as_bytes());
	w.u32(PART); // default params
	w.bytes(defaultparams.as_bytes());
	w.u32(PART); // instructions
	w.bytes(instructions.as_bytes());
	w.u32(PART); // sub-functions
	w.bytes(subfunctions.as_bytes());

	w.i64(stacksize as i64);
	w.u8(bgenerator);
	w.i64(varparams as i64);
	Ok(())
}

/// One serialized object: a 4-byte type tag (verbatim) then type-specific data
/// with widths widened.
fn transcode_object(r: &mut Reader, w: &mut Writer) -> TResult<()> {
	let tag = r.u32()?;
	w.u32(tag);
	match tag & RT_MASK {
		RT_NULL => {}
		RT_INTEGER | RT_BOOL => {
			w.i64(r.i32()? as i64); // sign-extend (ints may be negative)
		}
		RT_FLOAT => {
			// A float already in the literal table (e.g. from a 64-bit compile); BB's
			// own inline floats are hoisted here instead, by transcode_proto.
			w.f64(r.f32()? as f64); // value conversion, not bit-extension
		}
		RT_STRING => {
			let len = r.u32()?; // _len (SQInteger): 4 -> 8
			w.i64(len as i32 as i64);
			let s = r.block(len as usize)?; // string bytes (decrypted if >= 8)
			w.bytes(&s); // chars are 1 byte in both
		}
		other => return Err(format!("cannot transcode object of raw type {other:#x}")),
	}
	Ok(())
}

/// Read+verify a 4-byte marker and re-emit it.
fn expect_u32(r: &mut Reader, w: &mut Writer, expected: u32, what: &str) -> TResult<()> {
	let got = r.u32()?;
	if got != expected {
		return Err(format!(
			"expected {what} marker {expected:#010x}, got {got:#010x}"
		));
	}
	w.u32(got);
	Ok(())
}

/// Read+verify a 4-byte marker without emitting it (the caller writes section markers
/// itself when re-emitting buffered sections in stream order).
fn expect_marker(r: &mut Reader, expected: u32, what: &str) -> TResult<()> {
	let got = r.u32()?;
	if got != expected {
		return Err(format!(
			"expected {what} marker {expected:#010x}, got {got:#010x}"
		));
	}
	Ok(())
}

/// Read a 4-byte count (SQInteger in the 3.0.4 stream); return its value.
fn read_count(r: &mut Reader) -> TResult<usize> {
	let v = r.i32()?;
	usize::try_from(v).map_err(|_| format!("negative count in cnut: {v}"))
}

fn widen_unsigned(r: &mut Reader, w: &mut Writer) -> TResult<()> {
	w.u64(u64::from(r.u32()?));
	Ok(())
}

fn le_i32(b: &[u8]) -> i32 {
	i32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

/// Cursor over the encrypted cnut. Scalar reads are plaintext (the game never
/// encrypts < 8-byte writes); [`block`](Reader::block) reads a bulk chunk and
/// XXTEA-decrypts it.
struct Reader<'a> {
	data: &'a [u8],
	pos: usize,
}

impl<'a> Reader<'a> {
	fn new(data: &'a [u8]) -> Self {
		Reader { data, pos: 0 }
	}

	fn raw(&mut self, n: usize) -> TResult<&'a [u8]> {
		let end = self.pos.checked_add(n).ok_or("length overflow")?;
		if end > self.data.len() {
			return Err("unexpected end of cnut".into());
		}
		let s = &self.data[self.pos..end];
		self.pos = end;
		Ok(s)
	}

	fn u8(&mut self) -> TResult<u8> {
		Ok(self.raw(1)?[0])
	}
	fn u16(&mut self) -> TResult<u16> {
		let b = self.raw(2)?;
		Ok(u16::from_le_bytes([b[0], b[1]]))
	}
	fn u32(&mut self) -> TResult<u32> {
		let b = self.raw(4)?;
		Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
	}
	fn i32(&mut self) -> TResult<i32> {
		Ok(self.u32()? as i32)
	}
	fn f32(&mut self) -> TResult<f32> {
		let b = self.raw(4)?;
		Ok(f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
	}

	/// A bulk write chunk (one `SafeWrite`): read `n` bytes and XXTEA-decrypt
	/// (the cipher leaves chunks < 8 bytes, and the trailing `n % 4`, untouched).
	fn block(&mut self, n: usize) -> TResult<Vec<u8>> {
		let mut buf = self.raw(n)?.to_vec();
		crypt::decrypt_chunk(&mut buf);
		Ok(buf)
	}
}

/// Builds the output closure stream (always our 64-bit widths, little-endian).
struct Writer {
	out: Vec<u8>,
}

impl Writer {
	fn new() -> Self {
		Writer { out: Vec::new() }
	}
	fn u8(&mut self, v: u8) {
		self.out.push(v);
	}
	fn u16(&mut self, v: u16) {
		self.out.extend_from_slice(&v.to_le_bytes());
	}
	fn u32(&mut self, v: u32) {
		self.out.extend_from_slice(&v.to_le_bytes());
	}
	fn i64(&mut self, v: i64) {
		self.out.extend_from_slice(&v.to_le_bytes());
	}
	fn u64(&mut self, v: u64) {
		self.out.extend_from_slice(&v.to_le_bytes());
	}
	fn f64(&mut self, v: f64) {
		self.out.extend_from_slice(&v.to_le_bytes());
	}
	fn bytes(&mut self, b: &[u8]) {
		self.out.extend_from_slice(b);
	}
	fn as_bytes(&self) -> &[u8] {
		&self.out
	}
	fn into_bytes(self) -> Vec<u8> {
		self.out
	}
}
