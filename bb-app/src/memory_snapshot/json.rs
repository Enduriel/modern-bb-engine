//! The snapshot's JSON (`snapshot.json`): a small value type, an indented writer (the
//! file reads in a text editor too) and a parser (for the tests that read it back). Std
//! only.

use std::fmt::Write as _;

/// A JSON value. Objects keep their keys in the order written.
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
	Null,
	Bool(bool),
	Int(i64),
	Float(f64),
	Str(String),
	Array(Vec<Json>),
	Object(Vec<(String, Json)>),
}

impl From<&str> for Json {
	fn from(s: &str) -> Json {
		Json::Str(s.to_string())
	}
}

impl From<String> for Json {
	fn from(s: String) -> Json {
		Json::Str(s)
	}
}

impl From<u64> for Json {
	fn from(n: u64) -> Json {
		Json::Int(i64::try_from(n).unwrap_or(i64::MAX))
	}
}

impl From<usize> for Json {
	fn from(n: usize) -> Json {
		Json::from(n as u64)
	}
}

impl From<u32> for Json {
	fn from(n: u32) -> Json {
		Json::Int(i64::from(n))
	}
}

impl From<i64> for Json {
	fn from(n: i64) -> Json {
		Json::Int(n)
	}
}

impl From<f64> for Json {
	fn from(f: f64) -> Json {
		Json::Float(f)
	}
}

impl From<bool> for Json {
	fn from(b: bool) -> Json {
		Json::Bool(b)
	}
}

impl<T: Into<Json>> From<Option<T>> for Json {
	fn from(v: Option<T>) -> Json {
		v.map_or(Json::Null, Into::into)
	}
}

/// An object built field by field: `obj().with("a", 1u64).with("b", "x")`.
pub fn obj() -> Json {
	Json::Object(Vec::new())
}

impl Json {
	/// This object with `key` set to `value` (added last).
	pub fn with(mut self, key: &str, value: impl Into<Json>) -> Json {
		if let Json::Object(fields) = &mut self {
			fields.push((key.to_string(), value.into()));
		}
		self
	}

	/// The text, indented by tabs; a short array of numbers or strings stays on one line.
	pub fn to_text(&self) -> String {
		let mut out = String::new();
		self.write(&mut out, 0);
		out.push('\n');
		out
	}

	fn write(&self, out: &mut String, depth: usize) {
		match self {
			Json::Null => out.push_str("null"),
			Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
			Json::Int(n) => {
				let _ = write!(out, "{n}");
			}
			// JSON has no NaN or infinity.
			Json::Float(f) if !f.is_finite() => out.push_str("null"),
			Json::Float(f) => {
				let _ = write!(out, "{f}");
			}
			Json::Str(s) => write_str(out, s),
			Json::Array(items) if items.is_empty() => out.push_str("[]"),
			Json::Array(items) if items.iter().all(|i| matches!(i, Json::Int(_) | Json::Float(_) | Json::Str(_))) && items.len() <= 8 => {
				out.push('[');
				for (i, item) in items.iter().enumerate() {
					if i > 0 {
						out.push_str(", ");
					}
					item.write(out, depth);
				}
				out.push(']');
			}
			Json::Array(items) => {
				out.push('[');
				for (i, item) in items.iter().enumerate() {
					out.push_str(if i > 0 { ",\n" } else { "\n" });
					indent(out, depth + 1);
					item.write(out, depth + 1);
				}
				out.push('\n');
				indent(out, depth);
				out.push(']');
			}
			Json::Object(fields) if fields.is_empty() => out.push_str("{}"),
			Json::Object(fields) => {
				out.push('{');
				for (i, (key, value)) in fields.iter().enumerate() {
					out.push_str(if i > 0 { ",\n" } else { "\n" });
					indent(out, depth + 1);
					write_str(out, key);
					out.push_str(": ");
					value.write(out, depth + 1);
				}
				out.push('\n');
				indent(out, depth);
				out.push('}');
			}
		}
	}
}

impl Json {
	/// The value `text` holds.
	pub fn parse(text: &str) -> Result<Json, String> {
		let mut p = Parser { s: text.as_bytes(), i: 0 };
		let value = p.value()?;
		p.space();
		if p.i != p.s.len() {
			return Err(format!("text after the value at byte {}", p.i));
		}
		Ok(value)
	}

	/// An object's field `key`.
	pub fn get(&self, key: &str) -> Option<&Json> {
		match self {
			Json::Object(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
			_ => None,
		}
	}

	pub fn as_array(&self) -> Option<&[Json]> {
		match self {
			Json::Array(items) => Some(items),
			_ => None,
		}
	}

	pub fn as_str(&self) -> Option<&str> {
		match self {
			Json::Str(s) => Some(s),
			_ => None,
		}
	}

	pub fn as_i64(&self) -> Option<i64> {
		match self {
			Json::Int(n) => Some(*n),
			_ => None,
		}
	}
}

struct Parser<'a> {
	s: &'a [u8],
	i: usize,
}

impl Parser<'_> {
	fn space(&mut self) {
		while self.s.get(self.i).is_some_and(u8::is_ascii_whitespace) {
			self.i += 1;
		}
	}

	fn error<T>(&self, what: &str) -> Result<T, String> {
		Err(format!("{what} at byte {}", self.i))
	}

	fn eat(&mut self, word: &str) -> bool {
		let found = self.s[self.i..].starts_with(word.as_bytes());
		if found {
			self.i += word.len();
		}
		found
	}

	fn value(&mut self) -> Result<Json, String> {
		self.space();
		match self.s.get(self.i) {
			Some(b'{') => {
				self.i += 1;
				let mut fields = Vec::new();
				self.space();
				if self.eat("}") {
					return Ok(Json::Object(fields));
				}
				loop {
					self.space();
					let key = self.string()?;
					self.space();
					if !self.eat(":") {
						return self.error("no ':'");
					}
					fields.push((key, self.value()?));
					self.space();
					if self.eat("}") {
						return Ok(Json::Object(fields));
					}
					if !self.eat(",") {
						return self.error("no ',' or '}'");
					}
				}
			}
			Some(b'[') => {
				self.i += 1;
				let mut items = Vec::new();
				self.space();
				if self.eat("]") {
					return Ok(Json::Array(items));
				}
				loop {
					items.push(self.value()?);
					self.space();
					if self.eat("]") {
						return Ok(Json::Array(items));
					}
					if !self.eat(",") {
						return self.error("no ',' or ']'");
					}
				}
			}
			Some(b'"') => self.string().map(Json::Str),
			_ if self.eat("null") => Ok(Json::Null),
			_ if self.eat("true") => Ok(Json::Bool(true)),
			_ if self.eat("false") => Ok(Json::Bool(false)),
			_ => self.number(),
		}
	}

	fn number(&mut self) -> Result<Json, String> {
		let start = self.i;
		while self.s.get(self.i).is_some_and(|b| b.is_ascii_digit() || b"+-.eE".contains(b)) {
			self.i += 1;
		}
		let text = std::str::from_utf8(&self.s[start..self.i]).unwrap_or("");
		if let Ok(n) = text.parse::<i64>() {
			return Ok(Json::Int(n));
		}
		match text.parse::<f64>() {
			Ok(f) => Ok(Json::Float(f)),
			Err(_) => self.error("not a value"),
		}
	}

	fn string(&mut self) -> Result<String, String> {
		if !self.eat("\"") {
			return self.error("no string");
		}
		let mut out = Vec::new();
		loop {
			let Some(&b) = self.s.get(self.i) else { return self.error("an unended string") };
			self.i += 1;
			match b {
				b'"' => return String::from_utf8(out).map_err(|e| e.to_string()),
				b'\\' => {
					let Some(&e) = self.s.get(self.i) else { return self.error("an unended escape") };
					self.i += 1;
					match e {
						b'n' => out.push(b'\n'),
						b't' => out.push(b'\t'),
						b'r' => out.push(b'\r'),
						b'b' => out.push(8),
						b'f' => out.push(12),
						b'u' => {
							let hex = self.s.get(self.i..self.i + 4).and_then(|h| std::str::from_utf8(h).ok());
							let code = hex.and_then(|h| u32::from_str_radix(h, 16).ok());
							let Some(code) = code else { return self.error("a bad \\u escape") };
							self.i += 4;
							let c = char::from_u32(code).unwrap_or('\u{fffd}');
							out.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes());
						}
						other => out.push(other),
					}
				}
				b => out.push(b),
			}
		}
	}
}

fn indent(out: &mut String, depth: usize) {
	for _ in 0..depth {
		out.push('\t');
	}
}

fn write_str(out: &mut String, s: &str) {
	out.push('"');
	for c in s.chars() {
		match c {
			'"' => out.push_str("\\\""),
			'\\' => out.push_str("\\\\"),
			'\n' => out.push_str("\\n"),
			'\r' => out.push_str("\\r"),
			'\t' => out.push_str("\\t"),
			c if (c as u32) < 0x20 => {
				let _ = write!(out, "\\u{:04x}", c as u32);
			}
			c => out.push(c),
		}
	}
	out.push('"');
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn reads_back_what_it_writes() {
		let j = obj()
			.with("s", "tab\t \"q\" \u{e9}\u{1}")
			.with("n", -3i64)
			.with("f", 1.5f64)
			.with("a", Json::Array(vec![Json::Null, true.into(), obj()]))
			.with("e", Json::Array(Vec::new()));
		assert_eq!(Json::parse(&j.to_text()), Ok(j.clone()));
		assert_eq!(j.get("n").and_then(Json::as_i64), Some(-3));
		assert!(Json::parse("{\"a\": 1,}").is_err());
		assert!(Json::parse("[1] 2").is_err());
	}

	#[test]
	fn writes_indented_json() {
		let j = obj()
			.with("name", "a \"b\"\n")
			.with("n", 3u64)
			.with("f", f64::NAN)
			.with("list", Json::Array(vec![1u64.into(), 2u64.into()]))
			.with("rows", Json::Array(vec![obj().with("x", true)]))
			.with("none", None::<u64>);
		assert_eq!(
			j.to_text(),
			"{\n\t\"name\": \"a \\\"b\\\"\\n\",\n\t\"n\": 3,\n\t\"f\": null,\n\t\"list\": [1, 2],\n\t\"rows\": [\n\t\t{\n\t\t\t\"x\": true\n\t\t}\n\t],\n\t\"none\": null\n}\n"
		);
	}
}
