//! bb's output when it's run from a console. A GUI program gets no console of its own
//! (the point: none opens when the player double-clicks it), so started from one, it
//! attaches to its parent's and writes there. Caveats, as for any GUI program:
//! - cmd and PowerShell don't wait for a GUI program: the prompt comes back at once and
//!   bb's lines arrive after it (`start /wait bb.exe …` in cmd, or `bb.exe … | Out-Host`
//!   in PowerShell, waits). A program that started bb and waits for it (cargo, the
//!   tests, a script) sees it as before.
//! - Output redirected to a file or a pipe (`bb.exe --version > out.txt`, a parent's
//!   pipes, Wine's from a Linux shell) is kept as it is: only the standard handles
//!   that lead nowhere are pointed at the console.

use windows::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, HANDLE};
use windows::Win32::Storage::FileSystem::{
	CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_TYPE_CHAR, FILE_TYPE_DISK,
	FILE_TYPE_PIPE, GetFileType, OPEN_EXISTING,
};
use windows::Win32::System::Console::{
	ATTACH_PARENT_PROCESS, AttachConsole, CONSOLE_MODE, GetConsoleMode, GetStdHandle, STD_ERROR_HANDLE, STD_HANDLE,
	STD_OUTPUT_HANDLE, SetStdHandle,
};
use windows::core::w;

/// Whether the standard handle `id` already leads somewhere other than a console: a
/// file, a pipe, or a device such as `NUL`. Rust's `println!` and `eprintln!` look the
/// handle up at each write, so one set later is used from then on.
fn redirected(id: STD_HANDLE) -> bool {
	// SAFETY: plain queries of this process's standard handle.
	unsafe {
		let Ok(handle) = GetStdHandle(id) else { return false };
		if handle.is_invalid() || handle == HANDLE::default() {
			return false;
		}
		let mut mode = CONSOLE_MODE::default();
		match GetFileType(handle) {
			FILE_TYPE_DISK | FILE_TYPE_PIPE => true,
			FILE_TYPE_CHAR => GetConsoleMode(handle, &mut mode).is_err(),
			_ => false,
		}
	}
}

/// Attach to the console of the program that started bb, if it has one, and send
/// stdout and stderr there unless they're redirected. Call first thing in `main`, in
/// the game's own process (not CEF's child processes, which have nothing to say there).
pub fn attach() {
	let (out, err) = (redirected(STD_OUTPUT_HANDLE), redirected(STD_ERROR_HANDLE));
	if out && err {
		return;
	}
	// SAFETY: plain calls on this process's console and standard handles; the handle
	// opened is kept for the process's life.
	unsafe {
		// Fails when the parent has no console (a double-click: Explorer, Steam).
		if AttachConsole(ATTACH_PARENT_PROCESS).is_err() {
			return;
		}
		let Ok(console) = CreateFileW(
			w!("CONOUT$"),
			(GENERIC_READ | GENERIC_WRITE).0,
			FILE_SHARE_READ | FILE_SHARE_WRITE,
			None,
			OPEN_EXISTING,
			FILE_ATTRIBUTE_NORMAL,
			None,
		) else {
			return;
		};
		if !out {
			let _ = SetStdHandle(STD_OUTPUT_HANDLE, console);
		}
		if !err {
			let _ = SetStdHandle(STD_ERROR_HANDLE, console);
		}
	}
}
