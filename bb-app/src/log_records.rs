//! The `log` crate's warnings and errors in log.html and on stderr. wgpu and its
//! backends report what the system said only there: the `HRESULT` of a failed
//! `ResizeBuffers` or swapchain creation, a lost Vulkan surface. Without a logger they
//! were dropped, and a player's report of a failed surface carried only wgpu's
//! "Invalid surface".
//!
//! Each record is a row tagged with its crate (`wgpu_hal`, `wgpu_core`, …). After
//! [`MAX_ROWS`] the rest go to stderr only, so a warning repeated every frame can't
//! grow the log without end.

use std::sync::atomic::{AtomicUsize, Ordering};

use log::{Level, LevelFilter, Log, Metadata, Record};
use squirrel_bb::html_log::{self, Class};

/// The rows log.html takes from the `log` crate in a session.
pub const MAX_ROWS: usize = 200;

struct Forward {
	rows: AtomicUsize,
}

static FORWARD: Forward = Forward { rows: AtomicUsize::new(0) };

/// Send the `log` crate's warnings and errors to log.html and stderr from now on.
/// Only the first call sets the logger (the `log` crate takes one per process).
pub fn forward() {
	if log::set_logger(&FORWARD).is_ok() {
		log::set_max_level(LevelFilter::Warn);
	}
}

/// A record's row: its crate (the target's first path segment) and its text on one line.
pub fn row(target: &str, text: &str) -> (String, String) {
	let tag = target.split("::").next().unwrap_or(target).to_string();
	(tag, text.split_whitespace().collect::<Vec<_>>().join(" "))
}

impl Log for Forward {
	fn enabled(&self, metadata: &Metadata) -> bool {
		metadata.level() <= Level::Warn
	}

	fn log(&self, record: &Record) {
		if !self.enabled(record.metadata()) {
			return;
		}
		let (tag, text) = row(record.target(), &record.args().to_string());
		let class = if record.level() == Level::Error { Class::Error } else { Class::Warning };
		eprintln!("[{}] {tag}: {text}", record.level());
		let n = self.rows.fetch_add(1, Ordering::Relaxed);
		if n < MAX_ROWS {
			html_log::line(class, &tag, &text);
		} else if n == MAX_ROWS {
			html_log::line(Class::Warning, "Core", format!("More than {MAX_ROWS} warnings from the engine's libraries: the rest are on stderr only"));
		}
	}

	fn flush(&self) {}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_row_is_tagged_with_its_crate_on_one_line() {
		assert_eq!(
			row("wgpu_hal::dx12", "ResizeBuffers failed: 0x887A0001\n  more"),
			("wgpu_hal".into(), "ResizeBuffers failed: 0x887A0001 more".into())
		);
		assert_eq!(row("winit", "x"), ("winit".into(), "x".into()));
	}
}
