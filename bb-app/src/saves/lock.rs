//! One bb at a time on a saves folder: bb's default saves folder is the
//! original game's own, and two games writing the same saves would overwrite each
//! other's. A second bb on a folder another bb holds is told so and ends (`main.rs`);
//! bb's on different folders (`--saves`, `BB_SAVES_DIR`: the tests, a developer's runs)
//! and those with their saves in memory run side by side, as their logs do.
//!
//! The lock leaves no file anywhere, so nothing new shows in the folder (the original's
//! load menu lists only `*.sav`, but other programs and the player see the folder too),
//! and the system lets it go when its process ends, however it ends: a crash never
//! leaves one behind.
//! - **Linux, macOS:** an advisory lock (`flock`) on the folder itself, which is the same
//!   folder whatever path names it (a link, `..`). The original under Proton never takes
//!   one (Wine's file locks are `fcntl`'s, which `flock` doesn't see).
//! - **Windows:** a named mutex in the session's namespace, named after the folder's
//!   canonical path (lowercased: Windows' paths ignore case). Holding a lock on the folder
//!   itself would need a handle that shares nothing, which would keep the original and
//!   Explorer out of it.
//!
//! Neither handle is inherited by the processes bb starts (CEF's, a relaunch), so only
//! bb itself holds the lock.

use std::io;
use std::path::Path;

/// This bb's hold on its saves folder, while it lives.
#[derive(Debug)]
pub struct SavesLock {
	_held: Held,
}

impl SavesLock {
	/// Lock `dir` (created if missing) for this process: `None` while another process
	/// holds it.
	pub fn take(dir: &Path) -> io::Result<Option<SavesLock>> {
		std::fs::create_dir_all(dir)?;
		Ok(hold(dir)?.map(|held| SavesLock { _held: held }))
	}
}

#[cfg(unix)]
type Held = std::fs::File;

#[cfg(unix)]
fn hold(dir: &Path) -> io::Result<Option<Held>> {
	// Opened close-on-exec (std's default): a process bb starts doesn't keep it.
	let folder = std::fs::File::open(dir)?;
	match folder.try_lock() {
		Ok(()) => Ok(Some(folder)),
		Err(std::fs::TryLockError::WouldBlock) => Ok(None),
		Err(std::fs::TryLockError::Error(e)) => Err(e),
	}
}

#[cfg(windows)]
#[derive(Debug)]
struct Held(windows::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl Drop for Held {
	fn drop(&mut self) {
		// SAFETY: the mutex's handle, ours and still open.
		let _ = unsafe { windows::Win32::Foundation::CloseHandle(self.0) };
	}
}

#[cfg(windows)]
fn hold(dir: &Path) -> io::Result<Option<Held>> {
	use windows::Win32::Foundation::{CloseHandle, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, GetLastError};
	use windows::Win32::System::Threading::CreateMutexW;
	use windows::core::PCWSTR;
	let path = std::fs::canonicalize(dir)?;
	let name: Vec<u16> = mutex_name(&path.to_string_lossy()).encode_utf16().chain(std::iter::once(0)).collect();
	// SAFETY: a NUL-terminated name that outlives the call; no attributes (the handle
	// isn't inherited), not owned (its existence is the lock).
	match unsafe { CreateMutexW(None, false, PCWSTR(name.as_ptr())) } {
		// SAFETY: right after the call, on its thread; the handle is ours to close.
		Ok(handle) if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS => {
			let _ = unsafe { CloseHandle(handle) };
			Ok(None)
		}
		Ok(handle) => Ok(Some(Held(handle))),
		// One another user's process made, which this one may not open: held too.
		Err(e) if e.code() == ERROR_ACCESS_DENIED.to_hresult() => Ok(None),
		Err(e) => Err(io::Error::other(e)),
	}
}

/// The Windows mutex's name for the saves folder at the canonical `path`: its
/// lowercased UTF-16 hashed (FNV-1a, 64 bits), since a name can't hold `\` and is at
/// most `MAX_PATH` long. The same in every version of bb.
#[cfg_attr(not(windows), allow(dead_code))]
fn mutex_name(path: &str) -> String {
	let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
	for unit in path.to_lowercase().encode_utf16() {
		for byte in unit.to_le_bytes() {
			hash = (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
		}
	}
	format!("Local\\battle-brothers-modern-saves-{hash:016x}")
}

#[cfg(not(any(unix, windows)))]
type Held = ();

#[cfg(not(any(unix, windows)))]
fn hold(_dir: &Path) -> io::Result<Option<Held>> {
	Ok(Some(()))
}

#[cfg(test)]
mod tests {
	use std::io::{BufRead, BufReader, Write};
	use std::path::PathBuf;
	use std::process::{Command, Stdio};

	use super::*;

	/// The child process of [`a_crash_lets_the_lock_go`]: where it locks.
	const CHILD_DIR: &str = "BB_TEST_SAVES_LOCK_DIR";

	/// A folder of the test's own, emptied.
	fn folder(name: &str) -> PathBuf {
		let dir = std::env::temp_dir().join(format!("bb_saves_lock_{}_{name}", std::process::id()));
		let _ = std::fs::remove_dir_all(&dir);
		dir
	}

	/// A second process on the folder is refused while the first holds it.
	fn refused_elsewhere(dir: &Path) -> bool {
		let child = Command::new(std::env::current_exe().unwrap())
			.args(["--exact", "saves::lock::tests::child_holds_then_crashes", "--ignored", "--nocapture"])
			.env(CHILD_DIR, dir)
			.env("BB_TEST_SAVES_LOCK_PROBE", "1")
			.output()
			.expect("the test's child");
		String::from_utf8_lossy(&child.stdout).contains("refused")
	}

	#[test]
	fn one_holder_at_a_time_and_none_left_after() {
		let dir = folder("one");
		let held = SavesLock::take(&dir).unwrap().expect("a new folder's lock");
		assert!(dir.is_dir(), "the folder is made");
		assert!(refused_elsewhere(&dir), "another process took the held lock");
		// No file of the lock's in the folder.
		assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
		drop(held);
		assert!(!refused_elsewhere(&dir), "the lock outlived its holder");
		// Another folder is another lock.
		let _first = SavesLock::take(&dir).unwrap().expect("the folder's lock again");
		let other = folder("other");
		assert!(SavesLock::take(&other).unwrap().is_some(), "a second folder's lock");
		let _ = std::fs::remove_dir_all(&dir);
		let _ = std::fs::remove_dir_all(&other);
	}

	#[test]
	fn a_crash_lets_the_lock_go() {
		let dir = folder("crash");
		std::fs::create_dir_all(&dir).unwrap();
		let mut child = Command::new(std::env::current_exe().unwrap())
			.args(["--exact", "saves::lock::tests::child_holds_then_crashes", "--ignored", "--nocapture"])
			.env(CHILD_DIR, &dir)
			.stdin(Stdio::piped())
			.stdout(Stdio::piped())
			.spawn()
			.expect("the test's child");
		let mut out = BufReader::new(child.stdout.take().unwrap());
		let mut line = String::new();
		while !line.contains("held") {
			line.clear();
			assert!(out.read_line(&mut line).unwrap() > 0, "the child ended before it held the lock");
		}
		assert!(SavesLock::take(&dir).unwrap().is_none(), "the child's lock wasn't held");
		// It crashes (aborts) once told to.
		child.stdin.take().unwrap().write_all(b"crash\n").unwrap();
		let status = child.wait().unwrap();
		assert!(!status.success(), "the child didn't crash: {status}");
		assert!(SavesLock::take(&dir).unwrap().is_some(), "a crashed holder's lock stayed");
		let _ = std::fs::remove_dir_all(&dir);
	}

	/// Not a test: the other process of the two above. With `BB_TEST_SAVES_LOCK_PROBE`,
	/// it says whether the folder's lock is refused; else it holds the lock, says so,
	/// and crashes when told.
	#[test]
	#[ignore = "the child process of a_crash_lets_the_lock_go and one_holder_at_a_time_and_none_left_after"]
	fn child_holds_then_crashes() {
		// Nothing to do outside the two (`--include-ignored`).
		let Some(dir) = std::env::var_os(CHILD_DIR).map(PathBuf::from) else { return };
		let lock = SavesLock::take(&dir).unwrap();
		if std::env::var_os("BB_TEST_SAVES_LOCK_PROBE").is_some() {
			println!("{}", if lock.is_none() { "refused" } else { "taken" });
			return;
		}
		assert!(lock.is_some(), "the folder's lock was held");
		println!("held");
		std::io::stdout().flush().unwrap();
		let mut line = String::new();
		std::io::stdin().read_line(&mut line).unwrap();
		std::process::abort();
	}

	#[test]
	fn the_windows_name_ignores_case_and_holds_no_backslash() {
		let a = mutex_name(r"\\?\C:\Users\Player\Documents\Battle Brothers\savegames");
		let b = mutex_name(r"\\?\c:\users\player\documents\battle brothers\SAVEGAMES");
		assert_eq!(a, b);
		assert_ne!(a, mutex_name(r"\\?\C:\Users\Player\Documents\Battle Brothers\other"));
		assert!(a.starts_with("Local\\") && !a["Local\\".len()..].contains('\\') && a.len() < 260, "{a}");
	}
}
