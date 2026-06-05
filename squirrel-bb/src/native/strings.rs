//! BB string utilities: the `String` table (`contains`/`replace`/`remove`) and
//! the root helpers `buildTextFromTemplate` / `isFirstCharacter` /
//! `removeFromBeginningOfText` / `hexStringToInt`.
//!
//! Squirrel's stdlib `format`/`split`/`strip`/`lstrip`/`rstrip` come from the
//! string library the VM already registers — they are not re-implemented here.
//!
//! `toHash(obj)` (a per-object id) is in `native::identity`.

use squirrel::{SQVM, SqArray, SqRef, SqStrongRef, SqTable};

/// `%token%` substitution. `vars` is an array of `[name, value]` pairs; each
/// `%name%` in `template` is replaced with `value`.
///
/// Squirrel does not coerce numbers to strings on read, so the value is rendered
/// explicitly: scalars (int/float/bool/string) become their textual form;
/// containers/null are skipped (vanilla never passes those). This matters because
/// most vanilla vars are numbers — reward amounts, kill counts, etc.
fn build_text_from_template(template: &str, vars: &SqArray) -> String {
	let mut result = template.to_string();
	for index in 0..vars.len() {
		let Some(pair) = vars.get::<SqArray>(index) else {
			continue;
		};
		let Some(key) = pair.get::<String>(0) else {
			continue;
		};
		let value = match pair.get_ref(1) {
			Some(SqRef::String(s)) => s,
			Some(SqRef::Integer(n)) => n.to_string(),
			Some(SqRef::Float(f)) => f.to_string(),
			Some(SqRef::Bool(b)) => b.to_string(),
			_ => continue, // null / array / table / other: leave the token as-is
		};
		result = result.replace(&format!("%{key}%"), &value);
	}
	result
}

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

/// Register the string helpers and the `String` table.
pub(crate) fn install(vm: &mut SQVM) {
	// All `make_closure` calls (mutable VM borrow) happen before the table handles.
	let build = vm.make_closure(|template: &str, vars: SqArray| -> String {
		build_text_from_template(template, &vars)
	});
	let is_first = vm.make_closure(|text: &str, chars: SqArray| -> bool {
		// true if `text` starts with any of the strings in `chars` (vanilla passes
		// single characters, so this is a first-character membership test).
		chars.iter::<String>().any(|c| text.starts_with(&c))
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
