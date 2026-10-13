//! A small JSON value type with a writer and a parser: the wire format between the
//! game thread and the browser's JavaScript.
//!
//! Std only (no serde). Objects keep their key order, so a table serialized here
//! reaches JS with its keys in the order they were written.

use std::fmt::Write as _;

/// A JSON value.
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
	Null,
	Bool(bool),
	/// A number with no fraction or exponent in its text (or an integer written from
	/// Squirrel). JS has only doubles, but Squirrel tells integers apart.
	Int(i64),
	Float(f64),
	Str(String),
	Array(Vec<Json>),
	/// Key/value pairs in order (duplicate keys are kept; the last one wins in JS).
	Object(Vec<(String, Json)>),
}

impl Json {
	/// Serialize to compact JSON text.
	pub fn to_text(&self) -> String {
		let mut out = String::new();
		self.write(&mut out);
		out
	}

	/// Append compact JSON text to `out`.
	pub fn write(&self, out: &mut String) {
		match self {
			Json::Null => out.push_str("null"),
			Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
			Json::Int(n) => {
				let _ = write!(out, "{n}");
			}
			Json::Float(f) => write_float(*f, out),
			Json::Str(s) => write_string(s, out),
			Json::Array(items) => {
				out.push('[');
				for (i, item) in items.iter().enumerate() {
					if i > 0 {
						out.push(',');
					}
					item.write(out);
				}
				out.push(']');
			}
			Json::Object(entries) => {
				out.push('{');
				for (i, (key, value)) in entries.iter().enumerate() {
					if i > 0 {
						out.push(',');
					}
					write_string(key, out);
					out.push(':');
					value.write(out);
				}
				out.push('}');
			}
		}
	}

	/// Parse JSON text (the whole input must be one value, surrounding whitespace
	/// allowed).
	pub fn parse(text: &str) -> Result<Json, String> {
		let mut parser = Parser {
			bytes: text.as_bytes(),
			pos: 0,
		};
		let value = parser.value()?;
		parser.skip_ws();
		if parser.pos != parser.bytes.len() {
			return Err(parser.error("trailing characters"));
		}
		Ok(value)
	}

	/// The string, if this is one.
	pub fn as_str(&self) -> Option<&str> {
		match self {
			Json::Str(s) => Some(s),
			_ => None,
		}
	}

	/// The number as an `i64`, if this is an integral number.
	pub fn as_i64(&self) -> Option<i64> {
		match *self {
			Json::Int(n) => Some(n),
			Json::Float(f) if f.fract() == 0.0 && f.abs() < 9.0e15 => Some(f as i64),
			_ => None,
		}
	}

	/// The array items, if this is an array.
	pub fn as_array(&self) -> Option<&[Json]> {
		match self {
			Json::Array(items) => Some(items),
			_ => None,
		}
	}

	/// The value under `key`, if this is an object that has it (the last one wins).
	pub fn get(&self, key: &str) -> Option<&Json> {
		match self {
			Json::Object(entries) => entries.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v),
			_ => None,
		}
	}
}

/// A JS-compatible number: integral values print without a fraction, others with
/// the shortest text that round-trips. JSON has no NaN or infinity; like
/// `JSON.stringify`, those become `null`.
fn write_float(f: f64, out: &mut String) {
	if !f.is_finite() {
		out.push_str("null");
	} else if f.fract() == 0.0 && f.abs() < 1e21 {
		// `{}` would print 3.0 as "3"; keep it a plain integer literal.
		let _ = write!(out, "{}", f as i128);
	} else {
		let _ = write!(out, "{f}");
	}
}

fn write_string(s: &str, out: &mut String) {
	out.push('"');
	for c in s.chars() {
		match c {
			'"' => out.push_str("\\\""),
			'\\' => out.push_str("\\\\"),
			'\n' => out.push_str("\\n"),
			'\r' => out.push_str("\\r"),
			'\t' => out.push_str("\\t"),
			// Control characters, plus the two line terminators that are legal in JSON
			// but not in older JS string literals (the text is also eval'd as JS).
			c if (c as u32) < 0x20 || c == '\u{2028}' || c == '\u{2029}' => {
				let _ = write!(out, "\\u{:04x}", c as u32);
			}
			c => out.push(c),
		}
	}
	out.push('"');
}

struct Parser<'a> {
	bytes: &'a [u8],
	pos: usize,
}

impl Parser<'_> {
	fn error(&self, what: &str) -> String {
		format!("json: {what} at byte {}", self.pos)
	}

	fn skip_ws(&mut self) {
		while let Some(b' ' | b'\t' | b'\n' | b'\r') = self.bytes.get(self.pos) {
			self.pos += 1;
		}
	}

	fn peek(&self) -> Option<u8> {
		self.bytes.get(self.pos).copied()
	}

	fn expect(&mut self, b: u8) -> Result<(), String> {
		if self.peek() == Some(b) {
			self.pos += 1;
			Ok(())
		} else {
			Err(self.error(&format!("expected '{}'", b as char)))
		}
	}

	fn literal(&mut self, word: &str, value: Json) -> Result<Json, String> {
		if self.bytes[self.pos..].starts_with(word.as_bytes()) {
			self.pos += word.len();
			Ok(value)
		} else {
			Err(self.error("invalid literal"))
		}
	}

	fn value(&mut self) -> Result<Json, String> {
		self.skip_ws();
		match self.peek() {
			Some(b'n') => self.literal("null", Json::Null),
			Some(b't') => self.literal("true", Json::Bool(true)),
			Some(b'f') => self.literal("false", Json::Bool(false)),
			Some(b'"') => self.string().map(Json::Str),
			Some(b'[') => self.array(),
			Some(b'{') => self.object(),
			Some(b'-' | b'0'..=b'9') => self.number(),
			Some(_) => Err(self.error("unexpected character")),
			None => Err(self.error("unexpected end")),
		}
	}

	fn array(&mut self) -> Result<Json, String> {
		self.expect(b'[')?;
		let mut items = Vec::new();
		self.skip_ws();
		if self.peek() == Some(b']') {
			self.pos += 1;
			return Ok(Json::Array(items));
		}
		loop {
			items.push(self.value()?);
			self.skip_ws();
			match self.peek() {
				Some(b',') => self.pos += 1,
				Some(b']') => {
					self.pos += 1;
					return Ok(Json::Array(items));
				}
				_ => return Err(self.error("expected ',' or ']'")),
			}
		}
	}

	fn object(&mut self) -> Result<Json, String> {
		self.expect(b'{')?;
		let mut entries = Vec::new();
		self.skip_ws();
		if self.peek() == Some(b'}') {
			self.pos += 1;
			return Ok(Json::Object(entries));
		}
		loop {
			self.skip_ws();
			let key = self.string()?;
			self.skip_ws();
			self.expect(b':')?;
			let value = self.value()?;
			entries.push((key, value));
			self.skip_ws();
			match self.peek() {
				Some(b',') => self.pos += 1,
				Some(b'}') => {
					self.pos += 1;
					return Ok(Json::Object(entries));
				}
				_ => return Err(self.error("expected ',' or '}'")),
			}
		}
	}

	fn number(&mut self) -> Result<Json, String> {
		let start = self.pos;
		let mut integral = true;
		if self.peek() == Some(b'-') {
			self.pos += 1;
		}
		while let Some(b) = self.peek() {
			match b {
				b'0'..=b'9' => {}
				b'.' | b'e' | b'E' | b'+' | b'-' => integral = false,
				_ => break,
			}
			self.pos += 1;
		}
		// The slice is ASCII (digits and number punctuation only).
		let text = std::str::from_utf8(&self.bytes[start..self.pos]).expect("ascii");
		if integral {
			if let Ok(n) = text.parse::<i64>() {
				return Ok(Json::Int(n));
			}
		}
		text.parse::<f64>()
			.map(Json::Float)
			.map_err(|_| self.error("invalid number"))
	}

	fn hex4(&mut self) -> Result<u32, String> {
		let digits = self
			.bytes
			.get(self.pos..self.pos + 4)
			.and_then(|d| std::str::from_utf8(d).ok())
			.and_then(|d| u32::from_str_radix(d, 16).ok())
			.ok_or_else(|| self.error("invalid \\u escape"))?;
		self.pos += 4;
		Ok(digits)
	}

	fn string(&mut self) -> Result<String, String> {
		self.expect(b'"')?;
		let mut out = String::new();
		loop {
			// Copy the run up to the next quote or escape in one go.
			let run_start = self.pos;
			while let Some(b) = self.peek() {
				if b == b'"' || b == b'\\' {
					break;
				}
				self.pos += 1;
			}
			let run = std::str::from_utf8(&self.bytes[run_start..self.pos])
				.map_err(|_| self.error("invalid utf-8"))?;
			out.push_str(run);
			match self.peek() {
				Some(b'"') => {
					self.pos += 1;
					return Ok(out);
				}
				Some(b'\\') => {
					self.pos += 1;
					let escape = self
						.peek()
						.ok_or_else(|| self.error("unterminated escape"))?;
					self.pos += 1;
					match escape {
						b'"' => out.push('"'),
						b'\\' => out.push('\\'),
						b'/' => out.push('/'),
						b'b' => out.push('\u{8}'),
						b'f' => out.push('\u{c}'),
						b'n' => out.push('\n'),
						b'r' => out.push('\r'),
						b't' => out.push('\t'),
						b'u' => {
							let unit = self.hex4()?;
							let c = if (0xD800..0xDC00).contains(&unit)
								&& self.bytes[self.pos..].starts_with(b"\\u")
							{
								self.pos += 2;
								let low = self.hex4()?;
								char::from_u32(
									0x10000
										+ ((unit - 0xD800) << 10) + (low.wrapping_sub(0xDC00) & 0x3FF),
								)
							} else {
								char::from_u32(unit)
							};
							// A lone surrogate (legal in JS strings) has no `char`.
							out.push(c.unwrap_or('\u{FFFD}'));
						}
						_ => return Err(self.error("invalid escape")),
					}
				}
				_ => return Err(self.error("unterminated string")),
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::Json;

	#[test]
	fn round_trips_nested_values() {
		let text = r#"{"a":[1,null,2.5,"x\"y\n"],"b":{"c":true,"d":false},"e":-7}"#;
		let value = Json::parse(text).unwrap();
		assert_eq!(value.to_text(), text);
		assert_eq!(value.get("e"), Some(&Json::Int(-7)));
	}

	#[test]
	fn keeps_null_array_slots() {
		let value = Json::Array(vec![Json::Null, Json::Int(1), Json::Null]);
		assert_eq!(value.to_text(), "[null,1,null]");
	}

	#[test]
	fn floats_print_like_js() {
		assert_eq!(Json::Float(3.0).to_text(), "3");
		assert_eq!(Json::Float(0.5).to_text(), "0.5");
		assert_eq!(Json::Float(f64::NAN).to_text(), "null");
		assert_eq!(Json::Float(0.1f32 as f64).to_text(), "0.10000000149011612");
	}

	#[test]
	fn parses_unicode_escapes() {
		let value = Json::parse("\"\\u00e9\\ud83d\\ude00\"").unwrap();
		assert_eq!(value, Json::Str("\u{e9}\u{1f600}".to_string()));
		assert_eq!(Json::Str("\u{2028}".into()).to_text(), "\"\\u2028\"");
	}

	#[test]
	fn rejects_garbage() {
		assert!(Json::parse("{").is_err());
		assert!(Json::parse("[1,]").is_err());
		assert!(Json::parse("1 2").is_err());
	}
}
