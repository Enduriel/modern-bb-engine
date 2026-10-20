//! The run's record: one JSON object per line in `events.jsonl`, appended by the
//! supervisor and by each session (a whole line per write, so their lines never mix),
//! and the lines a session sends its supervisor on stdout.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// A JSON object, built field by field.
pub struct Json(String);

impl Json {
	pub fn new(kind: &str) -> Self {
		let ms = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
		let mut j = Json(String::from("{"));
		j.0.push_str(&format!("\"unix_ms\":{ms}"));
		let (session, seed) = SESSION.get().map(|s| (s.0, s.1.as_str())).unwrap_or((0, ""));
		j = j.int("session", session as i64).str("seed", seed);
		j.str("kind", kind)
	}

	pub fn str(mut self, key: &str, value: &str) -> Self {
		self.0.push_str(&format!(",{}:{}", quote(key), quote(value)));
		self
	}

	pub fn int(mut self, key: &str, value: i64) -> Self {
		self.0.push_str(&format!(",{}:{value}", quote(key)));
		self
	}

	pub fn num(mut self, key: &str, value: f64) -> Self {
		let v = if value.is_finite() { format!("{value}") } else { "null".into() };
		self.0.push_str(&format!(",{}:{v}", quote(key)));
		self
	}

	pub fn strs(mut self, key: &str, values: &[String]) -> Self {
		let list: Vec<String> = values.iter().map(|v| quote(v)).collect();
		self.0.push_str(&format!(",{}:[{}]", quote(key), list.join(",")));
		self
	}

	/// A field whose value is already JSON.
	pub fn raw(mut self, key: &str, json: &str) -> Self {
		self.0.push_str(&format!(",{}:{json}", quote(key)));
		self
	}

	pub fn finish(mut self) -> String {
		self.0.push('}');
		self.0
	}

	/// Append to the run's events file.
	pub fn emit(self) {
		let line = self.finish();
		if let Some(file) = EVENTS.get()
			&& let Ok(mut f) = file.lock()
		{
			let _ = f.write_all(format!("{line}\n").as_bytes());
		}
	}
}

/// `s` as a JSON string.
pub fn quote(s: &str) -> String {
	let mut out = String::with_capacity(s.len() + 2);
	out.push('"');
	for c in s.chars() {
		match c {
			'"' => out.push_str("\\\""),
			'\\' => out.push_str("\\\\"),
			'\n' => out.push_str("\\n"),
			'\r' => out.push_str("\\r"),
			'\t' => out.push_str("\\t"),
			c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
			c => out.push(c),
		}
	}
	out.push('"');
	out
}

/// At most `max` characters of `s`.
pub fn clip(s: &str, max: usize) -> String {
	match s.char_indices().nth(max) {
		Some((at, _)) => format!("{}…", &s[..at]),
		None => s.to_string(),
	}
}

static EVENTS: OnceLock<Mutex<File>> = OnceLock::new();
static SESSION: OnceLock<(u32, String)> = OnceLock::new();

/// Open the events file (appending) for this process, as session `session` (0 for the
/// supervisor) playing `seed`.
pub fn open(path: &Path, session: u32, seed: &str) -> std::io::Result<()> {
	let file = OpenOptions::new().create(true).append(true).open(path)?;
	let _ = EVENTS.set(Mutex::new(file));
	let _ = SESSION.set((session, seed.to_string()));
	Ok(())
}

/// A line to the supervisor (stdout): `SOAK <tag> <rest>`.
pub fn tell(tag: &str, rest: &str) {
	let mut out = std::io::stdout().lock();
	let _ = writeln!(out, "SOAK {tag} {}", rest.replace('\n', " "));
	let _ = out.flush();
}

/// Record every Rust panic (with its backtrace) the moment it happens: a panic inside a
/// native the VM called can't unwind through the C frames and aborts the process, so
/// this is the only chance. The session then ends (see `session::run`).
pub fn install_panic_hook() {
	std::panic::set_hook(Box::new(|info| {
		let message = info
			.payload()
			.downcast_ref::<&str>()
			.map(|s| s.to_string())
			.or_else(|| info.payload().downcast_ref::<String>().cloned())
			.unwrap_or_else(|| "(non-string panic)".into());
		let location = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
		let backtrace = std::backtrace::Backtrace::force_capture().to_string();
		eprintln!("PANIC {message} at {location}\n{backtrace}");
		tell("SIG", &format!("panic\t{}", signature(&format!("{message} at {location}"))));
		Json::new("panic")
			.str("message", &message)
			.str("location", &location)
			.str("backtrace", &clip(&backtrace, 20_000))
			.emit();
	}));
}

/// An error's signature, for grouping: digits and quoted values masked, clipped.
pub fn signature(s: &str) -> String {
	let mut out = String::new();
	let mut last_digit = false;
	for c in s.chars() {
		if c.is_ascii_digit() {
			if !last_digit {
				out.push('#');
			}
			last_digit = true;
		} else {
			last_digit = false;
			out.push(c);
		}
	}
	clip(&out.replace('\t', " ").replace('\n', " "), 300)
}

/// This process's resident memory (VmRSS) and its peak (VmHWM), in MB, from `status`
/// (a `/proc/<pid>/status`).
pub fn rss_mb(status: &str) -> (f64, f64) {
	let field = |name: &str| {
		status
			.lines()
			.find(|l| l.starts_with(name))
			.and_then(|l| l.split_whitespace().nth(1))
			.and_then(|v| v.parse::<f64>().ok())
			.map_or(0.0, |kb| kb / 1024.0)
	};
	(field("VmRSS:"), field("VmHWM:"))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn json_strings_are_escaped() {
		assert_eq!(quote("a\"b\\c\nd\u{1}"), "\"a\\\"b\\\\c\\nd\\u0001\"");
	}

	#[test]
	fn signatures_mask_numbers() {
		assert_eq!(signature("index 12 out of 3"), "index # out of #");
	}
}
