//! bb's log files in their folder ([`LogFiles::start`]): this session's `log.html` and
//! CEF's `cef.log`, and `logs/` with every earlier session's, each compressed into a zip
//! of its own (`logs/log-<start>.zip` holding just that `log.html`, and
//! `logs/cef-<start>.zip` holding that `cef.log`; `<start>` is when that session
//! started, in UTC, `2026-10-20T08-15-46.123Z`, so the names sort in order).
//!
//! The original overwrites its log at each start; we keep the earlier ones, robustly:
//! - **A crash** leaves its log in place; the next start archives it like any other.
//! - **Archiving never loses a log:** the zip is written under a temporary name, synced,
//!   read back and checked, and linked to its name; only then does the log go. If
//!   anything fails, the old log stays where it is and this session logs beside it (the
//!   next slot, below); a later start tries again.
//! - **Two instances at once:** each session holds a lock on its slot's lock file
//!   (`log.lock`; the next slots `log.2.lock`, … with `log.2.html`, `cef.2.log`). A start
//!   takes the first free slot, archives what an earlier session left in it, and also
//!   archives the leftovers of any other free slot. The lock file holds its session's
//!   start time, which names its archives.

mod archive;

use std::fs::{File, OpenOptions};
use std::io::{self, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub use archive::archive;

/// How many sessions may run at once.
const SLOTS: usize = 64;

/// One slot's files: the log, CEF's log and the lock.
struct Slot {
	html: PathBuf,
	cef: PathBuf,
	lock: PathBuf,
}

impl Slot {
	fn new(dir: &Path, n: usize) -> Slot {
		if n == 0 {
			Slot { html: dir.join("log.html"), cef: dir.join("cef.log"), lock: dir.join("log.lock") }
		} else {
			let n = n + 1;
			Slot { html: dir.join(format!("log.{n}.html")), cef: dir.join(format!("cef.{n}.log")), lock: dir.join(format!("log.{n}.lock")) }
		}
	}

	/// Open and lock the slot's lock file: `None` while another session holds it.
	fn lock(&self) -> io::Result<Option<File>> {
		let file = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(&self.lock)?;
		match file.try_lock() {
			Ok(()) => Ok(Some(file)),
			Err(std::fs::TryLockError::WouldBlock) => Ok(None),
			Err(std::fs::TryLockError::Error(e)) => Err(e),
		}
	}

	/// Archive what an earlier session left here (held: `lock`); the archives made, or
	/// what went wrong, naming the file it went wrong with.
	fn archive_leftovers(&self, lock: &mut File, logs: &Path) -> Result<Vec<PathBuf>, String> {
		let recorded = archive::read_all(lock).unwrap_or_default();
		let mut made = Vec::new();
		for (file, kind, entry) in [(&self.html, "log", "log.html"), (&self.cef, "cef", "cef.log")] {
			let Ok(meta) = std::fs::metadata(file) else { continue };
			let stamp = Some(recorded.trim())
				.filter(|s| !s.is_empty())
				.map(str::to_string)
				.unwrap_or_else(|| stamp(meta.created().or_else(|_| meta.modified()).unwrap_or_else(|_| SystemTime::now())));
			let kept = if meta.len() == 0 {
				std::fs::remove_file(file).map(|()| None)
			} else {
				archive(file, logs, kind, &stamp, entry).map(Some)
			};
			made.extend(kept.map_err(|e| format!("{}: {e}", file.display()))?);
		}
		Ok(made)
	}
}

/// `time` as an archive's name has it: UTC, to the millisecond, sorting in order.
pub fn stamp(time: SystemTime) -> String {
	let Ok(ts) = jiff::Timestamp::try_from(time) else { return "unknown".into() };
	let utc = ts.to_zoned(jiff::tz::TimeZone::UTC);
	format!("{}.{:03}Z", utc.strftime("%Y-%m-%dT%H-%M-%S"), utc.millisecond())
}

/// This session's log files, in `dir`. Holds the slot's lock while it lives.
pub struct LogFiles {
	/// Where this session's log.html goes.
	pub html: PathBuf,
	/// Where CEF's log goes.
	pub cef: PathBuf,
	/// The archives this start made of earlier sessions' logs.
	pub archived: Vec<PathBuf>,
	/// What went wrong archiving (the logs left in place), for the caller to report.
	pub problems: Vec<String>,
	_lock: File,
}

impl LogFiles {
	/// Take the first free slot in `dir` (created if missing), archive the logs earlier
	/// sessions left (see the module), and record this session's start.
	pub fn start(dir: &Path) -> io::Result<LogFiles> {
		std::fs::create_dir_all(dir)?;
		let logs = dir.join("logs");
		archive::remove_stale_temps(&logs);
		let started = stamp(SystemTime::now());
		let (mut archived, mut problems) = (Vec::new(), Vec::new());
		let mut ours = None;
		for n in 0..SLOTS {
			let slot = Slot::new(dir, n);
			// Past our slot, only those an earlier session left logs in.
			if ours.is_some() && !slot.html.exists() && !slot.cef.exists() {
				continue;
			}
			let Some(mut lock) = slot.lock()? else { continue };
			match slot.archive_leftovers(&mut lock, &logs) {
				Ok(made) => archived.extend(made),
				Err(e) => {
					// Its logs stay; a later start tries again.
					problems.push(e);
					continue;
				}
			}
			if ours.is_none() {
				lock.set_len(0)?;
				lock.seek(SeekFrom::Start(0))?;
				lock.write_all(started.as_bytes())?;
				lock.sync_all()?;
				ours = Some((slot, lock));
			}
		}
		let (slot, lock) = ours.ok_or_else(|| io::Error::other(format!("no free log slot in {}", dir.display())))?;
		Ok(LogFiles { html: slot.html, cef: slot.cef, archived, problems, _lock: lock })
	}
}

#[cfg(test)]
mod tests;
