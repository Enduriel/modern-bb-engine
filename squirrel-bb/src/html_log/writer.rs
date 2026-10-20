//! [`HtmlLog`]: one log.html being written.

use std::fs::File;
use std::io::{self, Write};
use std::path::Path;

use super::format;

/// A log.html open for writing. Each row goes to the file as it's written (the file is
/// unbuffered, so tools reading the log while the game runs see every line, as the
/// original's flushes each); dropping it closes the page.
pub struct HtmlLog {
	file: File,
}

impl HtmlLog {
	/// Start a new log at `path` (replacing a file there), with `title` as the page's
	/// title and header.
	pub fn create(path: &Path, title: &str) -> io::Result<HtmlLog> {
		let mut file = File::create(path)?;
		file.write_all(format::header(title).as_bytes())?;
		Ok(HtmlLog { file })
	}

	/// Write `bytes` (whole rows) to the file.
	pub fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
		self.file.write_all(bytes)
	}
}

impl Drop for HtmlLog {
	fn drop(&mut self) {
		let _ = self.file.write_all(format::FOOTER.as_bytes());
		let _ = self.file.sync_all();
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::html_log::format::{Class, row};

	#[test]
	fn rows_reach_the_file_at_once_and_the_page_closes_on_drop() {
		let dir = std::env::temp_dir().join(format!("bb-html-log-{}", std::process::id()));
		std::fs::create_dir_all(&dir).unwrap();
		let path = dir.join("log.html");
		let mut log = HtmlLog::create(&path, "Battle Brothers 1.5.2.3").unwrap();
		log.write(&row(Class::Info, "10:00:00", "SQ", b"one")).unwrap();
		// Readable while the game runs: no buffering in between.
		let open = std::fs::read_to_string(&path).unwrap();
		assert!(open.starts_with("<html><head><title>Battle Brothers 1.5.2.3</title>"));
		assert!(open.ends_with("<div class=\"text\">one</div></div></div>"));
		drop(log);
		let closed = std::fs::read_to_string(&path).unwrap();
		assert_eq!(closed, format!("{open}</div></body></html>"));
		std::fs::remove_dir_all(&dir).unwrap();
	}
}
