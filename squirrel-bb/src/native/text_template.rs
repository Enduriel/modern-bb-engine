//! `buildTextFromTemplate(text, vars)`, as the original's (`0x505e40`, disassembled):
//! the scripts' texts with random choices and variables, e.g.
//! `"{You saved %name%'s life | %name% owes you}"`.
//!
//! 1. Choices (`0x505ba0`, recursive): everything from the first `{` on is replaced by
//!    the resolved rest (so a later or nested choice is drawn first); then the text up
//!    to the first `}` is split on `" | "` and one option, picked at random, replaces it
//!    and the `}`. One option draws nothing. Options aren't trimmed, `{a|b}` is one
//!    option, and an unmatched `{` is just removed.
//! 2. The split (`0x504530`) keeps the text after the last `" | "` only if it is at least
//!    2 bytes long: `{aa | b}` always gives `aa`. A 1-byte choice like `{x}` has no
//!    options at all, and the original then indexes an empty vector (undefined, likely a
//!    crash); here it resolves to nothing. No vanilla text has either.
//! 3. Each var's `%key%` is replaced everywhere, in the array's order, left to right and
//!    without re-scanning what it inserted; keys and values print as Squirrel's
//!    `tostring` does (an integer `%d`, a float `%g` of its `f32`, `true`/`false`, null
//!    as `(null : 0x00000000)`). A missing var leaves its `%key%`.
//! 4. One leading and one trailing `"\n\n"` are removed.
//!
//! The picks draw from a stream of the original's own (a `mt19937_64` seeded from the OS
//! at startup, never by scripts), so they can't be reproduced and touch no other stream.

use squirrel::{SqArray, SqRef};

use super::EngineRng;

/// `0x504530`'s split on `" | "`.
fn split(s: &[u8]) -> Vec<&[u8]> {
	const DELIM: &[u8] = b" | ";
	let mut out = Vec::new();
	let mut pos = 0;
	while let Some(i) = s[pos..].windows(DELIM.len()).position(|w| w == DELIM) {
		out.push(&s[pos..pos + i]);
		pos += i + DELIM.len();
	}
	// The tail only if it's at least 2 bytes (an unsigned `pos < len - 1`).
	if (pos as u64) < (s.len() as u64).wrapping_sub(1) {
		out.push(&s[pos..]);
	}
	out
}

/// `0x505ba0`: resolve the choices in `s`.
fn resolve(s: &[u8], rng: &EngineRng) -> Vec<u8> {
	let mut s = s.to_vec();
	if let Some(p) = s.iter().position(|&b| b == b'{') {
		let rest = resolve(&s[p + 1..], rng);
		s.truncate(p);
		s.extend_from_slice(&rest);
	}
	let Some(q) = s.iter().position(|&b| b == b'}') else { return s };
	let options = split(&s[..q]);
	let pick = match options.len() {
		0 => Vec::new(),
		1 => options[0].to_vec(),
		n => options[rng.uniform_index(n as u64) as usize].to_vec(),
	};
	let mut out = pick;
	out.extend_from_slice(&s[q + 1..]);
	out
}

/// A value as Squirrel's `tostring` prints it in the original (`0x58ac40`).
fn to_text(v: &SqRef) -> Vec<u8> {
	match v {
		SqRef::String(s) => s.as_bytes().to_vec(),
		SqRef::Integer(n) => n.to_string().into_bytes(),
		SqRef::Float(f) => super::save::meta_format_float(*f as f32).into_bytes(),
		SqRef::Bool(b) => b.to_string().into_bytes(),
		SqRef::Null => b"(null : 0x00000000)".to_vec(),
		// The original prints the object's type and address, which can't be reproduced.
		_ => b"(object : 0x00000000)".to_vec(),
	}
}

/// `0x504630`: replace every `from` in `s`, left to right, resuming after each insert.
fn replace_all(s: &mut Vec<u8>, from: &[u8], to: &[u8]) {
	if from.is_empty() || s.is_empty() {
		return;
	}
	let mut pos = 0;
	while let Some(i) = s[pos..].windows(from.len()).position(|w| w == from) {
		let at = pos + i;
		s.splice(at..at + from.len(), to.iter().copied());
		pos = at + to.len();
	}
}

/// `buildTextFromTemplate(text, vars)`: `vars` is an array of `[key, value]` pairs.
pub(crate) fn build(text: &[u8], vars: &SqArray, rng: &EngineRng) -> Vec<u8> {
	if text.is_empty() {
		return Vec::new();
	}
	let mut s = resolve(text, rng);
	for index in 0..vars.len() {
		let Some(pair) = vars.get::<SqArray>(index) else { continue };
		let (Some(key), Some(value)) = (pair.get_ref(0), pair.get_ref(1)) else { continue };
		let mut token = b"%".to_vec();
		token.extend(to_text(&key));
		token.push(b'%');
		replace_all(&mut s, &token, &to_text(&value));
	}
	if s.starts_with(b"\n\n") {
		s.drain(..2);
	}
	if s.ends_with(b"\n\n") {
		s.truncate(s.len() - 2);
	}
	s
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Choices resolved with a stream seeded so that its picks are known.
	fn resolved(text: &str) -> String {
		String::from_utf8(resolve(text.as_bytes(), &EngineRng::seeded(1))).unwrap()
	}

	#[test]
	fn choices_resolve_as_the_originals() {
		assert_eq!(resolved("{You saved Grimald's life.}"), "You saved Grimald's life.");
		assert_eq!(resolved("{aa | b}"), "aa");
		assert_eq!(resolved("{a|b}"), "a|b");
		assert_eq!(resolved("{}"), "");
		assert_eq!(resolved("a } b"), "a  b");
		assert_eq!(resolved("a {bb"), "a bb");
		assert_eq!(resolved("{x}"), "");
		let picked = resolved("{aa | bb | cc}");
		assert!(["aa", "bb", "cc"].contains(&picked.as_str()), "{picked}");
		// Nested: the inner choice resolves into the outer one's options.
		let nested = resolved("{xx | {yy | zz} q}");
		assert!(["xx", "yy q", "zz q"].contains(&nested.as_str()), "{nested}");
	}

	#[test]
	fn the_split_keeps_a_tail_of_two_bytes_or_more() {
		assert_eq!(split(b"aa | bb"), [&b"aa"[..], &b"bb"[..]]);
		assert_eq!(split(b"aa | b"), [&b"aa"[..]]);
		assert!(split(b"x").is_empty());
		assert_eq!(split(b""), [&b""[..]]);
	}

	#[test]
	fn replacing_doesnt_rescan_what_it_inserted() {
		let mut s = b"%a% %A%".to_vec();
		replace_all(&mut s, b"%a%", b"%A%");
		replace_all(&mut s, b"%A%", b"x");
		assert_eq!(s, b"x x");
	}
}
