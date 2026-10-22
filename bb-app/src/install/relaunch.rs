//! Starting bb again, as the player started it, once they chose the game's folder: the
//! browser host that showed the not-found window can't start again in this process,
//! and the game needs it.
//!
//! On Linux and macOS the process becomes the new bb (`exec`: the same process id, so
//! whatever started bb, Steam or a script, sees it run on). On Windows, which has no
//! `exec`, the new bb is this one's child, and this one waits for it and ends with its
//! exit code.

use std::ffi::OsString;
use std::process::Command;

/// Start bb again with this run's arguments and `extra` after them. Returns only on
/// Windows (the new bb's exit code) or when it couldn't start (why).
pub fn relaunch(extra: &[OsString]) -> Result<i32, String> {
	let exe = std::env::current_exe().map_err(|e| format!("where bb is: {e}"))?;
	let mut command = Command::new(&exe);
	command.args(std::env::args_os().skip(1)).args(extra);
	eprintln!("starting the game again: {}", exe.display());
	run(command).map_err(|e| format!("starting {} again: {e}", exe.display()))
}

#[cfg(unix)]
fn run(mut command: Command) -> std::io::Result<i32> {
	use std::os::unix::process::CommandExt;
	// Only returns if it failed.
	Err(command.exec())
}

#[cfg(not(unix))]
fn run(mut command: Command) -> std::io::Result<i32> {
	Ok(command.status()?.code().unwrap_or(1))
}
