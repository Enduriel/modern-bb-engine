//! log.html's markup, byte for byte as the original writes it (its format strings, read
//! from the executable, and its log files): the page's head, a row a line, and a script
//! error's row with its call stack. Text is written as given: the original escapes
//! nothing (scripts log HTML on purpose: Modern Hooks and MSU colour their lines), and
//! turns a line's newlines into `<br>` (not in a stack's values).

use squirrel::RuntimeError;

use super::value;

/// The original's stylesheet, verbatim.
pub const CSS: &str = "body{background-color:#000000;margin:0;font-family:Arial;font-style:normal;font-size:62.5%}*{-webkit-box-sizing:border-box;-moz-box-sizing:border-box;box-sizing:border-box}.header{background-color:#000000;box-shadow:0 -6px 0 rgba(0,0,0,0.2) inset;width:100%;height:4rem;margin-bottom:.4rem;color:#d97e1c;font-size:12px;text-align:center;line-height:4rem}.content{width:94%;margin:0 auto}.row{background-color:#000000;width:100%;float:left;margin-bottom:.2rem;padding-left:.4rem;padding-right:.4rem;padding-bottom:.2rem;font-size:12px;line-height:2rem;box-shadow:0 -4px 0 rgba(0,0,0,0.2) inset}.row.debug{color:#ffcc42}.row.info{color:#2d8cff}.row.warning{color:#ff6000}.row.error{color:#a00}.row.critical{color:#990aaa}.row .entry-container{width:100%}.row .time{width:12rem;height:2.0rem;float:left}.row .tag{width:14rem;height:2.0rem;float:left}.row .text{float:none;overflow:hidden}.row .stacktrace-container{width:100%;padding:.4rem}.row .value-container{background-color:#222;width:100%;margin-bottom:.4rem;padding:.4rem}.row .function-container{width:100%}.row .label{color:#b67745;width:6rem;height:2.0rem;float:left}.row .value{color:#95addd;float:none;overflow:hidden} .valueVar{color:#4874cb;float:none;overflow:hidden}";

/// What closes the page when the game quits cleanly (a crashed session's log lacks it).
pub const FOOTER: &str = "</div></body></html>";

/// A row's class: how the original's log sorts its lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
	Debug,
	Info,
	Warning,
	Error,
	Critical,
}

impl Class {
	pub fn name(self) -> &'static str {
		match self {
			Class::Debug => "debug",
			Class::Info => "info",
			Class::Warning => "warning",
			Class::Error => "error",
			Class::Critical => "critical",
		}
	}
}

/// The page up to its first row; `title` is both the page's title and its header.
pub fn header(title: &str) -> String {
	format!(
		"<html><head><title>{title}</title><style type=\"text/css\">{CSS}</style></head><body><div class=\"header\">{title}</div><div class=\"content\">"
	)
}

/// `text` with each newline as `<br>`.
fn with_breaks(text: &[u8]) -> Vec<u8> {
	let mut out = Vec::with_capacity(text.len());
	for &b in text {
		if b == b'\n' {
			out.extend_from_slice(b"<br>");
		} else {
			out.push(b);
		}
	}
	out
}

/// A row's opening, up to the end of its text.
fn entry(out: &mut Vec<u8>, class: Class, time: &str, tag: &str, text: &[u8]) {
	out.extend_from_slice(b"<div class=\"row ");
	out.extend_from_slice(class.name().as_bytes());
	out.extend_from_slice(b"\"><div class=\"entry-container\"><div class=\"time\">");
	out.extend_from_slice(time.as_bytes());
	out.extend_from_slice(b"</div><div class=\"tag\">");
	out.extend_from_slice(tag.as_bytes());
	out.extend_from_slice(b"</div><div class=\"text\">");
	out.extend_from_slice(&with_breaks(text));
	out.extend_from_slice(b"</div></div>");
}

/// One line: `time` is the wall clock (`HH:MM:SS`), `tag` who logged it (`SQ` for the
/// scripts' `logInfo` and kin).
pub fn row(class: Class, time: &str, tag: &str, text: &[u8]) -> Vec<u8> {
	let mut out = Vec::with_capacity(text.len() + 160);
	entry(&mut out, class, time, tag, text);
	out.extend_from_slice(b"</div>");
	out
}

/// An uncaught script error: an error row tagged `Script Error`, its message as the
/// text, then each frame (innermost first) with its locals. `message` is the thrown
/// string (the original writes no row for anything else thrown).
pub fn script_error(time: &str, message: &[u8], error: &RuntimeError) -> Vec<u8> {
	let mut out = Vec::new();
	entry(&mut out, Class::Error, time, "Script Error", message);
	out.extend_from_slice(b"<div class=\"stacktrace-container\"><div class=\"value-container\">");
	for frame in &error.frames {
		let function = frame.function.as_deref().unwrap_or("<Unknown>");
		out.extend_from_slice(b"<div class=\"function-container\"><div class=\"label\">Function:</div><div class=\"value\">");
		match &frame.source {
			Some(source) => out.extend_from_slice(format!("{function} -> {source} : {}", frame.line).as_bytes()),
			None => out.extend_from_slice(function.as_bytes()),
		}
		out.extend_from_slice(b"</div></div>");
		if !frame.locals.is_empty() {
			out.extend_from_slice(b"<div class=\"function-container\"><div class=\"label\">Variables:</div><div class=\"valueVar\">");
			for (i, local) in frame.locals.iter().enumerate() {
				if i > 0 {
					out.extend_from_slice(b", ");
				}
				out.extend_from_slice(local.name.as_bytes());
				out.extend_from_slice(b" = ");
				out.extend_from_slice(&value::local(&local.value));
			}
			out.extend_from_slice(b"</div></div>");
		}
	}
	out.extend_from_slice(b"</div></div></div>");
	out
}

#[cfg(test)]
mod tests {
	use squirrel::{ErrorValue, Frame, Local};

	use super::*;

	fn text(bytes: Vec<u8>) -> String {
		String::from_utf8(bytes).unwrap()
	}

	#[test]
	fn the_head_is_the_originals() {
		// The start of the original's log.html (1.5.2.3), up to the stylesheet's start.
		let head = header("Battle Brothers 1.5.2.3");
		assert!(head.starts_with(
			"<html><head><title>Battle Brothers 1.5.2.3</title><style type=\"text/css\">body{background-color:#000000;margin:0;font-family:Arial;"
		));
		assert!(head.ends_with(
			".row .value{color:#95addd;float:none;overflow:hidden} .valueVar{color:#4874cb;float:none;overflow:hidden}</style></head><body><div class=\"header\">Battle Brothers 1.5.2.3</div><div class=\"content\">"
		));
	}

	#[test]
	fn rows_as_the_original_writes_them() {
		// Rows from the original's log (probe 2026-10-20): nothing is escaped, a newline
		// becomes <br>, a tab stays.
		assert_eq!(
			text(row(Class::Info, "10:22:26", "SQ", b"PROBE_LOG info a\nb <b>bold</b> & &amp; \"q\" 'a' \t tab \\ end")),
			"<div class=\"row info\"><div class=\"entry-container\"><div class=\"time\">10:22:26</div><div class=\"tag\">SQ</div><div class=\"text\">PROBE_LOG info a<br>b <b>bold</b> & &amp; \"q\" 'a' \t tab \\ end</div></div></div>"
		);
		assert_eq!(
			text(row(Class::Error, "10:22:26", "Script", b"PROBE_LOG error()")),
			"<div class=\"row error\"><div class=\"entry-container\"><div class=\"time\">10:22:26</div><div class=\"tag\">Script</div><div class=\"text\">PROBE_LOG error()</div></div></div>"
		);
		assert_eq!(
			text(row(Class::Critical, "20:46:01", "", b"A critical error occured: x")),
			"<div class=\"row critical\"><div class=\"entry-container\"><div class=\"time\">20:46:01</div><div class=\"tag\"></div><div class=\"text\">A critical error occured: x</div></div></div>"
		);
		// Bytes go out as they are (the original writes a script's UTF-8 or cp1252 as is).
		assert!(row(Class::Debug, "t", "SQ", b"\xc3\xa9 \xe9").windows(4).any(|w| w == b"\xc3\xa9 \xe9"));
	}

	#[test]
	fn a_script_error_as_the_original_writes_it() {
		let local = |name: &str, value| Local { name: name.into(), value };
		let error = RuntimeError {
			error: ErrorValue::String(b"the index 'foo' does not exist".to_vec()),
			frames: vec![
				Frame {
					function: Some("outer".into()),
					source: Some("bb_probe/probe.nut".into()),
					line: 36,
					locals: vec![
						local("thrower", ErrorValue::Closure),
						local("q", ErrorValue::Integer(7)),
						local("_x", ErrorValue::Float(1.25)),
						local("this", ErrorValue::Table),
					],
				},
				Frame {
					function: None,
					source: Some("bb_probe/probe.nut".into()),
					line: 40,
					locals: vec![local("_d", ErrorValue::Null), local("s", ErrorValue::String(b"a\nb".to_vec()))],
				},
				Frame { function: Some("unknown".into()), source: Some("NATIVE".into()), line: -1, locals: vec![] },
			],
		};
		// The original's row for such a stack (pieced from its log: probe 2026-10-20, and
		// a native frame from a modded run).
		assert_eq!(
			text(script_error("10:22:27", b"the index 'foo' does not exist", &error)),
			"<div class=\"row error\"><div class=\"entry-container\"><div class=\"time\">10:22:27</div><div class=\"tag\">Script Error</div><div class=\"text\">the index 'foo' does not exist</div></div>\
			<div class=\"stacktrace-container\"><div class=\"value-container\">\
			<div class=\"function-container\"><div class=\"label\">Function:</div><div class=\"value\">outer -> bb_probe/probe.nut : 36</div></div>\
			<div class=\"function-container\"><div class=\"label\">Variables:</div><div class=\"valueVar\">thrower = Function, q = 7, _x = 1.25, this = Table</div></div>\
			<div class=\"function-container\"><div class=\"label\">Function:</div><div class=\"value\"><Unknown> -> bb_probe/probe.nut : 40</div></div>\
			<div class=\"function-container\"><div class=\"label\">Variables:</div><div class=\"valueVar\">_d = Null, s = a\nb</div></div>\
			<div class=\"function-container\"><div class=\"label\">Function:</div><div class=\"value\">unknown -> NATIVE : -1</div></div>\
			</div></div></div>"
		);
	}
}
