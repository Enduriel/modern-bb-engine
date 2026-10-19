//! What the page and the bridge reported during a session ([`PageLog`]), and where it is
//! echoed as it arrives ([`Echo`]).

use std::collections::VecDeque;

/// How many lines a [`PageLog`] keeps.
const CAPACITY: usize = 2000;

/// One line of what the page or the bridge reported.
#[derive(Clone, Debug)]
pub struct LogLine {
	/// `"error"`, `"warning"`, `"info"` (page console) or `"bridge"`.
	pub level: String,
	pub text: String,
}

impl LogLine {
	/// An error on the page, or the bridge reporting something gone wrong.
	pub fn is_problem(&self) -> bool {
		self.level == "error" || self.level == "bridge"
	}
}

/// Where a session's page and bridge lines go as they arrive, besides its log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Echo {
	/// Nowhere.
	Off,
	/// Errors, warnings and bridge lines to stderr; an uncaught script error prints its
	/// call stack there too. For a player's terminal.
	Problems,
	/// Every line to stdout; the scripts' `print` output and uncaught errors (with their
	/// call stack) to stderr. For debugging.
	All,
}

impl Echo {
	/// Echo `line` as this says.
	pub(super) fn line(self, line: &LogLine) {
		match self {
			Echo::Off => {}
			Echo::Problems => {
				if line.is_problem() || line.level == "warning" {
					eprintln!("page {}: {}", line.level, line.text);
				}
			}
			Echo::All => println!("  [{}] {}", line.level, line.text),
		}
	}
}

/// The most recent lines, a bounded number of them: a long session reports without end
/// (mods' warnings, say). When it is full, the oldest line that isn't a problem goes
/// first, so chatter can't push out an earlier error.
#[derive(Debug, Default)]
pub(super) struct PageLog {
	lines: VecDeque<LogLine>,
}

impl PageLog {
	pub(super) fn push(&mut self, line: LogLine) {
		self.push_within(line, CAPACITY);
	}

	fn push_within(&mut self, line: LogLine, capacity: usize) {
		if self.lines.len() >= capacity {
			let oldest = self.lines.iter().position(|l| !l.is_problem()).unwrap_or(0);
			self.lines.remove(oldest);
		}
		self.lines.push_back(line);
	}

	pub(super) fn lines(&self) -> Vec<LogLine> {
		self.lines.iter().cloned().collect()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn line(level: &str, text: &str) -> LogLine {
		LogLine { level: level.into(), text: text.into() }
	}

	fn texts(log: &PageLog) -> Vec<String> {
		log.lines().into_iter().map(|l| l.text).collect()
	}

	#[test]
	fn keeps_the_latest_lines_in_order() {
		let mut log = PageLog::default();
		for i in 0..5 {
			log.push_within(line("info", &i.to_string()), 3);
		}
		assert_eq!(texts(&log), ["2", "3", "4"]);
	}

	#[test]
	fn chatter_goes_before_problems() {
		let mut log = PageLog::default();
		log.push_within(line("error", "e"), 3);
		log.push_within(line("info", "a"), 3);
		log.push_within(line("bridge", "b"), 3);
		log.push_within(line("warning", "w"), 3);
		assert_eq!(texts(&log), ["e", "b", "w"]);
		log.push_within(line("error", "f"), 3);
		assert_eq!(texts(&log), ["e", "b", "f"]);
		// Only problems left: the oldest goes.
		log.push_within(line("error", "g"), 3);
		assert_eq!(texts(&log), ["b", "f", "g"]);
	}
}
