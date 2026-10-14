//! Squirrel values ↔ [`Json`]: how script data crosses to the browser and back, with
//! the original's conversions (probed 2026-10-13).
//!
//! SQ → JS:
//! - tables become objects; only string keys cross, and entries JS can't hold
//!   (closures, classes, instances, …) are left out. Such a value passed on its own
//!   arrives as `undefined`. In an array it crashes the original ("Array size
//!   mismatch"), so no script does that; we send `null`;
//! - arrays become arrays and **keep their `null` slots** (screens use fixed positions,
//!   e.g. empty paperdoll slots);
//! - floats arrive as the original's 32-bit floats widened (`0.1` →
//!   `0.10000000149011612`), which our f32 math mode already stores;
//! - strings are decoded as UTF-8 (probed 2026-10-14, `probes/text_bytes.*`): a string
//!   that isn't valid UTF-8 (`0xE9.tochar()`, Windows-1252's é) arrives **empty**, and
//!   one with a NUL byte is cut off there. Table keys are assumed to go the same way
//!   (unprobed).
//!
//! JS → SQ:
//! - strings arrive as UTF-8, cut off at a NUL (probed 2026-10-14). A lone UTF-16
//!   surrogate ends the original ("string too long"); here it's U+FFFD (`json.rs`);
//! - **every number becomes an integer, truncated toward zero** (`2.7` → `2`; the
//!   original converts with a 32-bit `cvttsd2si`, so out-of-range values and NaN
//!   become `-2147483648`). Mods that need fractions send strings (MSU does);
//! - objects become tables, arrays arrays; the page's bridge script already turned
//!   `undefined` into `null` and functions into empty arrays, as the original does.

use squirrel::{SQVM, SqArray, SqRef, SqStrongRef, SqTable};

use crate::json::Json;

/// How deep a value is followed before it's cut off as `null` (guards against cyclic
/// tables; UI data is a few levels deep).
const MAX_DEPTH: usize = 64;

/// The JSON form of a retained Squirrel value; `None` for one JS can't hold (which
/// reaches JS as `undefined`).
pub fn strong_to_json(vm: &SQVM, value: &SqStrongRef) -> Option<Json> {
	convert(&value.get_value::<SqRef>(vm)?, 0)
}

/// The JSON form of a Squirrel value (`null` for one JS can't hold).
pub fn to_json(value: &SqRef) -> Json {
	convert(value, 0).unwrap_or(Json::Null)
}

/// The JSON form of a value, or `None` for one that doesn't cross.
fn convert(value: &SqRef, depth: usize) -> Option<Json> {
	if depth > MAX_DEPTH {
		return Some(Json::Null);
	}
	Some(match value {
		SqRef::Null => Json::Null,
		SqRef::Integer(n) => Json::Int(*n),
		SqRef::Float(f) => Json::Float(*f),
		SqRef::Bool(b) => Json::Bool(*b),
		SqRef::String(_) | SqRef::Bytes(_) => Json::Str(text(value)?),
		SqRef::Array(array) => Json::Array(
			(0..array.len())
				.map(|i| {
					array
						.get_ref(i)
						.and_then(|item| convert(&item, depth + 1))
						.unwrap_or(Json::Null)
				})
				.collect(),
		),
		SqRef::Table(table) => Json::Object(
			table
				.entries()
				.into_iter()
				.filter_map(|(key, value)| Some((text(&key)?, convert(&value, depth + 1)?)))
				.collect(),
		),
		_ => return None,
	})
}

/// The page's string for a script string, as the original converts it: up to the first
/// NUL, and empty if it isn't UTF-8. `None` if `value` isn't a string.
fn text(value: &SqRef) -> Option<String> {
	match value {
		SqRef::String(s) => Some(up_to_nul(s).to_string()),
		SqRef::Bytes(_) => Some(String::new()),
		_ => None,
	}
}

/// `s` up to its first NUL: the original's strings end there both ways.
fn up_to_nul(s: &str) -> &str {
	s.split('\0').next().unwrap_or_default()
}

/// A JS number as the original hands it to a script: truncated toward zero to a
/// 32-bit integer (`cvttsd2si`), whose "integer indefinite" result for NaN and
/// out-of-range values is `-2147483648`.
fn js_number(f: f64) -> i64 {
	if f.is_finite() && f > -2_147_483_649.0 && f < 2_147_483_648.0 {
		f.trunc() as i64
	} else {
		i32::MIN as i64
	}
}

/// Build the Squirrel value for `json` (new tables/arrays in `vm`).
pub fn from_json<'vm>(vm: &'vm SQVM, json: &Json) -> SqRef<'vm> {
	match json {
		Json::Null => SqRef::Null,
		Json::Bool(b) => SqRef::Bool(*b),
		Json::Int(n) => SqRef::Integer(js_number(*n as f64)),
		Json::Float(f) => SqRef::Integer(js_number(*f)),
		Json::Str(s) => SqRef::String(up_to_nul(s).to_string()),
		Json::Array(items) => {
			let mut array = SqArray::new(vm);
			for item in items {
				array.push(from_json(vm, item));
			}
			SqRef::Array(array)
		}
		Json::Object(entries) => {
			let mut table = SqTable::new(vm);
			for (key, value) in entries {
				table.set(up_to_nul(key), from_json(vm, value));
			}
			SqRef::Table(table)
		}
	}
}

#[cfg(test)]
mod tests {
	use squirrel::{SQVM, SqStrongRef, SqTable};

	use super::{from_json, strong_to_json, to_json};
	use crate::json::Json;

	fn global(vm: &SQVM, name: &str) -> SqStrongRef {
		SqTable::root(vm).get::<SqStrongRef>(name).expect("global")
	}

	#[test]
	fn arrays_keep_null_slots_and_tables_become_objects() {
		let vm = SQVM::new(1024);
		vm.run_script(r#"::v <- [null, 1, 2.5, "x", { a = true }, null];"#, "t")
			.unwrap();
		let json = strong_to_json(&vm, &global(&vm, "v")).unwrap();
		assert_eq!(json.to_text(), r#"[null,1,2.5,"x",{"a":true},null]"#);
	}

	#[test]
	fn strings_cross_as_utf8_up_to_a_nul() {
		let vm = SQVM::new(1024);
		vm.run_script(
			r#"local c = @(n) n.tochar(); ::v <- [c(0xC3) + c(0xA9), c(0xE9), "a" + c(0) + "b", "x"];"#,
			"t",
		)
		.unwrap();
		let json = strong_to_json(&vm, &global(&vm, "v")).unwrap();
		assert_eq!(json.to_text(), r#"["é","","a","x"]"#);
	}

	#[test]
	fn page_strings_end_at_a_nul() {
		let vm = SQVM::new(1024);
		let json = Json::parse(r#"["a\u0000b", "\u00e9"]"#).unwrap();
		let mut root = SqTable::root(&vm);
		root.set("t", from_json(&vm, &json));
		assert_eq!(strong_to_json(&vm, &global(&vm, "t")).unwrap().to_text(), r#"["a","é"]"#);
	}

	#[test]
	fn only_string_keys_and_plain_values_cross() {
		let vm = SQVM::new(1024);
		vm.run_script(
			r#"::v <- { f = function() {}, [3] = "three", s = "kept" };"#,
			"t",
		)
		.unwrap();
		let json = strong_to_json(&vm, &global(&vm, "v")).unwrap();
		assert_eq!(json.to_text(), r#"{"s":"kept"}"#);
	}

	#[test]
	fn js_numbers_arrive_as_truncated_integers() {
		let vm = SQVM::new(1024);
		let json =
			Json::parse(r#"[7, [1, 2.0, 2.7, -2.7], {"name": "x", "n": null}, 1e10]"#).unwrap();
		let value = from_json(&vm, &json);
		SqTable::root(&vm).set("w", value);
		vm.run_script(
			r#"
			assert(typeof ::w[0] == "integer");
			assert(::w[1][2] == 2 && typeof ::w[1][2] == "integer");
			assert(::w[1][3] == -2);
			assert(::w[2].name == "x" && ::w[2].n == null);
			assert(::w[3] == -2147483648);
			"#,
			"t",
		)
		.unwrap();
		let back = to_json(&SqTable::root(&vm).get_ref("w").unwrap());
		assert_eq!(back.as_array().unwrap()[1].to_text(), "[1,2,2,-2]");
	}
}
