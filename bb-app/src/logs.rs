//! `bb`'s log files ([`Logs`]): log.html in the original's format, and CEF's cef.log
//! beside it, in `battle-brothers-modern/` in the system's data folder (or `--logs DIR`);
//! the earlier sessions' kept as zips in `logs/` there (see `bb_engine::log_files`).

use std::path::{Path, PathBuf};

use bb_engine::log_files::LogFiles;
use squirrel_bb::html_log::{self, Class};

/// The page's title and header, as the original's (its version, which the scripts see).
const TITLE: &str = "Battle Brothers 1.5.2.3";

/// The session's log files, open until [`Logs::close`] (or a drop).
pub struct Logs {
	files: LogFiles,
}

impl Logs {
	/// Start this session's log in `dir`, archiving the earlier sessions' there; the
	/// first lines are the original's (its write path) and ours (the engine's version).
	/// Problems archiving are reported (the old logs stay where they were).
	pub fn start(dir: &Path) -> Result<Logs, String> {
		let files = LogFiles::start(dir).map_err(|e| format!("the log folder {}: {e}", dir.display()))?;
		html_log::open(&files.html, TITLE).map_err(|e| format!("{}: {e}", files.html.display()))?;
		html_log::line(Class::Info, "Core", format!("Using write path: {}", dir.display()));
		html_log::line(Class::Info, "Core", format!("Engine: battle-brothers-modern {}", env!("CARGO_PKG_VERSION")));
		for problem in &files.problems {
			html_log::engine(Class::Warning, "Core", &format!("An earlier log could not be archived and was left in place: {problem}"));
		}
		eprintln!("log: {}", files.html.display());
		// What the engine's libraries warn of (wgpu's backends' system errors among it).
		crate::log_records::forward();
		// A panic ends the game: the original's critical line, then the usual report.
		let report = std::panic::take_hook();
		std::panic::set_hook(Box::new(move |info| {
			html_log::critical(&info.to_string());
			report(info);
		}));
		Ok(Logs { files })
	}

	/// The folder of the logs.
	pub fn dir(&self) -> Option<&Path> {
		self.files.html.parent()
	}

	/// Where CEF writes its log.
	pub fn cef_log(&self) -> PathBuf {
		self.files.cef.clone()
	}

	/// The session ends: a fatal `error` gets the original's critical line, then the
	/// page is closed.
	pub fn close(self, error: Option<&str>) {
		if let Some(error) = error {
			html_log::critical(error);
		}
		html_log::close();
	}
}

impl Drop for Logs {
	fn drop(&mut self) {
		html_log::close();
	}
}
