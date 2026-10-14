//! BB string utilities: the `String` table (`contains`/`replace`/`remove`) and
//! the root helpers `buildTextFromTemplate` (see [`super::text_template`]) /
//! `isFirstCharacter` / `removeFromBeginningOfText` / `hexStringToInt`.
//!
//! Squirrel's stdlib `format`/`split`/`strip`/`lstrip`/`rstrip` come from the
//! string library the VM already registers — they are not re-implemented here.
//!
//! `toHash(obj)` (a per-object id) is in `native::identity`.

use squirrel::{SQVM, SqArray, SqBytes, SqRef, SqStrongRef, SqTable};

use super::{EngineRng, text_template};

/// Parse a hex string (tolerating a leading `0x`/`0X`/`#`) into an integer; `0`
/// on malformed input.
fn parse_hex(text: &str) -> i64 {
	let text = text.trim();
	let digits = text
		.strip_prefix("0x")
		.or_else(|| text.strip_prefix("0X"))
		.or_else(|| text.strip_prefix('#'))
		.unwrap_or(text);
	u64::from_str_radix(digits, 16)
		.map(|v| v as i64)
		.unwrap_or(0)
}

/// Register the string helpers and the `String` table. `buildTextFromTemplate` picks
/// its random choices from `text_rng`.
pub(crate) fn install(vm: &mut SQVM, text_rng: EngineRng) {
	// All `make_closure` calls (mutable VM borrow) happen before the table handles.
	let build = vm.make_closure(move |template: SqBytes, vars: SqArray| -> SqBytes {
		SqBytes(text_template::build(&template.0, &vars, &text_rng))
	});
	let is_first = vm.make_closure(|text: &str, chars: SqRef| -> bool {
		// true if `text` starts with any of the strings in `chars` (vanilla passes
		// single characters, so this is a first-character membership test). Anything
		// but an array is false, as in the original (parity probe `menu/strings`).
		match chars {
			SqRef::Array(chars) => chars.iter::<String>().any(|c| text.starts_with(&c)),
			_ => false,
		}
	});
	let remove_begin = vm.make_closure(|prefix: &str, text: &str| -> String {
		text.strip_prefix(prefix).unwrap_or(text).to_string()
	});
	let hex = vm.make_closure(|text: &str| -> i64 { parse_hex(text) });

	let contains = vm.make_closure(|text: &str, needle: &str| -> bool { text.contains(needle) });
	let replace = vm.make_closure(|text: &str, from: &str, to: &str| -> String {
		if from.is_empty() {
			text.to_string()
		} else {
			text.replace(from, to)
		}
	});
	let remove = vm.make_closure(|text: &str, substr: &str| -> String {
		if substr.is_empty() {
			text.to_string()
		} else {
			text.replace(substr, "")
		}
	});

	let mut root = SqTable::root(vm);
	root.set("buildTextFromTemplate", build);
	root.set("isFirstCharacter", is_first);
	root.set("removeFromBeginningOfText", remove_begin);
	root.set("hexStringToInt", hex);

	let mut string_table = SqTable::new(vm);
	let string_fns: [(&str, SqStrongRef); 3] = [
		("contains", contains),
		("replace", replace),
		("remove", remove),
	];
	for (name, func) in string_fns {
		string_table.set(name, func);
	}
	root.set("String", string_table);
}
