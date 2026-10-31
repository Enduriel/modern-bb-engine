//! log.html: the log the original keeps (`Documents/Battle Brothers/log.html`), in its
//! format, for players, modders and their tools ([`format`] has the markup). Ours lives
//! in our own folder (`battle-brothers-modern/` in the system's data folder, see
//! [`platform::logs_dir`](crate::platform::logs_dir)), never the original's.
//!
//! The log is process-wide: the scripts' `logInfo` and kin, uncaught script errors, the
//! engine's own lines and the page's errors all go to the one open [`HtmlLog`]. Without
//! one open (tests, tools) writing is a no-op; the terminal output is separate.
//!
//! Paths in it never show the user's home folder ([`anonymise`]).

pub mod anonymise;
pub mod format;
pub mod value;
mod writer;

use std::io;
use std::path::Path;
use std::sync::Mutex;

pub use format::Class;
use squirrel::{ErrorValue, RuntimeError};
pub use writer::HtmlLog;

static LOG: Mutex<Option<HtmlLog>> = Mutex::new(None);

fn with_log(f: impl FnOnce(&mut HtmlLog)) {
	let mut log = LOG.lock().unwrap_or_else(|e| e.into_inner());
	if let Some(log) = log.as_mut() {
		f(log);
	}
}

/// The wall clock as the original's rows show it (`%H:%M:%S`, local time).
pub fn now() -> String {
	jiff::Zoned::now().strftime("%H:%M:%S").to_string()
}

/// Make the log at `path` (a new file) the one written to, closing any before it.
pub fn open(path: &Path, title: &str) -> io::Result<()> {
	let log = HtmlLog::create(path, title)?;
	*LOG.lock().unwrap_or_else(|e| e.into_inner()) = Some(log);
	Ok(())
}

/// Write `folder` in the open log's rows as `placeholder` from now on (a folder outside
/// the home folder that would show the user's name: a Documents moved to another
/// drive), unless it's inside the home folder or no folder of its own.
pub fn hide(folder: &Path, placeholder: &str) {
	with_log(|log| log.hide(folder, placeholder));
}

/// Close the log (its page ends, as the original's does at a clean quit).
pub fn close() {
	LOG.lock().unwrap_or_else(|e| e.into_inner()).take();
}

/// Whether a log is open.
pub fn is_open() -> bool {
	LOG.lock().unwrap_or_else(|e| e.into_inner()).is_some()
}

/// Write a row: `tag` is who logged it (the original's: `SQ` for the scripts' log
/// functions, `Script` for the script system, `UI`, `Core`, …).
pub fn line(class: Class, tag: &str, text: impl AsRef<[u8]>) {
	with_log(|log| {
		let _ = log.write(&format::row(class, &now(), tag, text.as_ref()));
	});
}

/// Write an uncaught script error with its call stack. Only a thrown string makes a
/// row, as in the original (which logs nothing for a thrown table or number).
pub fn script_error(error: &RuntimeError) {
	let ErrorValue::String(message) = &error.error else { return };
	with_log(|log| {
		let _ = log.write(&format::script_error(&now(), message, error));
	});
}

/// An engine line: to log.html under `tag`, and to the terminal as the scripts' lines
/// go there (`[Warning] …` on stderr).
pub fn engine(class: Class, tag: &str, text: &str) {
	line(class, tag, text);
	match class {
		Class::Debug | Class::Info => println!("[{}] {text}", capitalized(class)),
		_ => eprintln!("[{}] {text}", capitalized(class)),
	}
}

fn capitalized(class: Class) -> &'static str {
	match class {
		Class::Debug => "Debug",
		Class::Info => "Info",
		Class::Warning => "Warning",
		Class::Error => "Error",
		Class::Critical => "Critical",
	}
}

/// Write the original's line for a fatal error, as the game ends. Safe from a panic
/// hook: if the panic came while the log was being written, the line is left out.
pub fn critical(message: &str) {
	let text = format!("A critical error occured: {message}");
	let mut log = match LOG.try_lock() {
		Ok(log) => log,
		Err(std::sync::TryLockError::Poisoned(e)) => e.into_inner(),
		Err(std::sync::TryLockError::WouldBlock) => return,
	};
	if let Some(log) = log.as_mut() {
		let _ = log.write(&format::row(Class::Critical, &now(), "", text.as_bytes()));
	}
}
