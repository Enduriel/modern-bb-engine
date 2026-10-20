//! The string library and lexer behave as Squirrel 3.0's, the original engine's.
//! Every expected value here was read from the original game.

use super::SQVM;

/// Run `checks` after a prelude that defines `enc(v)` (a value as text, strings with
/// every non-printable byte as `\xNN`) and `eq(label, got, want)` (throws on mismatch).
fn run(checks: &str) {
	let prelude = r#"
		local hex = "0123456789ABCDEF";
		::enc <- function(v) {
			switch (typeof v) {
			case "string":
				local o = "\"";
				for (local i = 0; i < v.len(); i++) {
					local c = v[i] & 0xFF;
					if (c >= 32 && c < 127 && c != 92 && c != 34) o += c.tochar();
					else o += "\\x" + hex[c >> 4].tochar() + hex[c & 15].tochar();
				}
				return o + "\"";
			case "array":
				local o = "[";
				foreach (i, e in v) o += (i > 0 ? "," : "") + ::enc(e);
				return o + "]";
			default: return typeof v + ":" + v;
			}
		};
		::eq <- function(label, f, want) {
			local got;
			try { got = ::enc(f()); } catch (e) { got = "ERROR: " + e; }
			if (got != want) throw label + ": got " + got + ", want " + want;
		};
	"#;
	let vm = SQVM::new(1024);
	vm.run_script(prelude, "prelude").expect("prelude");
	if let Err(e) = vm.run_script(checks, "checks") {
		panic!("{e}");
	}
}

#[test]
fn split_skips_empty_tokens_like_strtok() {
	run(r#"
		// mod_plan_perks' perk string: "°" is two UTF-8 bytes, each a separator.
		::eq("perks", @() split("a#2°b#2°", "°"), "[\"a#2\",\"b#2\"]");
		::eq("a,,b", @() split("a,,b", ","), "[\"a\",\"b\"]");
		::eq(",a,", @() split(",a,", ","), "[\"a\"]");
		::eq("empty", @() split("", ","), "[]");
		::eq("only seps", @() split(",,,", ","), "[]");
		::eq("multi", @() split("a b;c;; d", " ;"), "[\"a\",\"b\",\"c\",\"d\"]");
		::eq("no sep", @() split("abc", ","), "[\"abc\"]");
		::eq("all sep chars", @() split("ab", "ab"), "[]");
		::eq("empty seps", @() split("abc", ""), "ERROR: empty separators string");
		::eq("3 args", @() split("a,,b", ",", true), "ERROR: wrong number of parameters");
		::eq("len", @() split("x,,y,", ",").len(), "integer:2");
	"#);
}

#[test]
fn split_strings_end_at_nul() {
	// Not probed directly (the probe's NUL literals hit the \x rule below), but
	// the original runs strtok on a C copy of both strings.
	run(r#"
		::eq("nul in str", @() split("a\0b,c", ","), "[\"a\"]");
		::eq("nul in seps", @() split("a,b;c", ",\0;"), "[\"a\",\"b;c\"]");
	"#);
}

#[test]
fn hex_escapes_read_up_to_four_digits() {
	run(r#"
		::eq("3 digits", @() "\xB0b", "\"\\x0B\"");
		::eq("perk bytes", @() split("a#2\xC2\xB0b#2\xC2\xB0", "\xC2\xB0"), "[\"a#2\",\"\\x0B#2\"]");
		::eq("4 digits", @() "\x0041", "\"A\"");
		::eq("5 digits", @() "\x00411", "\"A1\"");
		::eq("2 digits", @() "\x41z", "\"Az\"");
	"#);
}

#[test]
fn format_matches_squirrel_3_0() {
	run(r#"
		::eq("bool d", @() format("%d", true), "ERROR: integer expected for the specified format");
		::eq("bool f", @() format("%f", false), "ERROR: float expected for the specified format");
		::eq("missing", @() format("%d"), "ERROR: not enough paramters for the given format string");
		::eq("nul in fmt", @() format("a\0%d", 5), "\"a\"");
		::eq("d float", @() format("%d", 3.7), "\"3\"");
		::eq("widths", @() format("[%10s][%-10s][%.3s]", "ab", "ab", "abcdef"), "\"[        ab][ab        ][abc]\"");
	"#);
}
