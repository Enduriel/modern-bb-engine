//! A native crash in the game's process (an access violation in a driver, CEF or the
//! VM's C code: an exception nothing handles) in log.html and in the message box, as a
//! panic is ([`failure::show_panics`](crate::failure::show_panics)). Without it the log
//! just stops, and the player sees the window vanish.
//!
//! The line has the exception's code and where it happened, as the module and the
//! offset in it (an address alone moves with each start). Then the exception goes on
//! to the system's error reporting as before: Windows' Event Viewer gets its
//! "Application Error" entry, and a dump where the player has turned them on
//! (Windows Error Reporting's `LocalDumps`). A filter installed after this one (CEF's
//! crash reporter, were it on) replaces it.
//!
//! Best effort: the process is in an unknown state. The line is written only if the
//! log isn't being written right then ([`html_log::critical`]), and the message box is
//! shown once, whatever crashed first.

use std::path::PathBuf;
use std::sync::OnceLock;

use windows::Win32::Foundation::HMODULE;
use windows::Win32::System::Diagnostics::Debug::{
	EXCEPTION_POINTERS, LPTOP_LEVEL_EXCEPTION_FILTER, SetUnhandledExceptionFilter,
};
use windows::Win32::System::LibraryLoader::{
	GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, GetModuleFileNameW,
	GetModuleHandleExW,
};
use windows::core::PCWSTR;

/// The filter's answer that lets the system go on with the exception.
const EXCEPTION_CONTINUE_SEARCH: i32 = 0;

/// The log's folder, for the message box, and the filter this one replaced.
static INSTALLED: OnceLock<(Option<PathBuf>, LPTOP_LEVEL_EXCEPTION_FILTER)> = OnceLock::new();

/// Report the process's unhandled exceptions from now on (`logs`: the log's folder,
/// which the message box names). Call once, in the game's own process.
pub fn report_crashes(logs: Option<PathBuf>) {
	// SAFETY: `filter` is a filter as the call expects, for the process's life.
	let previous = unsafe { SetUnhandledExceptionFilter(Some(filter)) };
	let _ = INSTALLED.set((logs, previous));
}

/// The module `address` is in and the offset in it: `bb.exe+0x1234`, or the address
/// alone outside any.
fn place(address: usize) -> String {
	let mut module = HMODULE::default();
	let flags = GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT;
	// SAFETY: with FROM_ADDRESS the "name" is any address, only looked up; the module
	// isn't held (UNCHANGED_REFCOUNT), and is only named.
	unsafe {
		if GetModuleHandleExW(flags, PCWSTR(address as *const u16), &mut module).is_err() {
			return format!("{address:#x}");
		}
		let mut name = [0u16; 260];
		let len = GetModuleFileNameW(Some(module), &mut name) as usize;
		let path = String::from_utf16_lossy(&name[..len.min(name.len())]);
		let file = path.rsplit(['\\', '/']).next().unwrap_or(&path);
		format!("{file}+{:#x}", address.wrapping_sub(module.0 as usize))
	}
}

/// What the log and the box say of exception `code` at `address`, on `thread`.
pub fn describe(code: u32, at: &str, thread: &str) -> String {
	let what = match code {
		0xC000_0005 => " (an access violation)",
		0xC000_001D => " (an illegal instruction)",
		0xC000_00FD => " (a stack overflow)",
		0xC000_0094 => " (an integer division by zero)",
		0xC000_0409 => " (a fast fail: the program stopped itself)",
		0xE06D_7363 => " (a C++ exception nothing caught)",
		_ => "",
	};
	format!("a system exception {code:#010x}{what} at {at}, on the thread '{thread}'")
}

unsafe extern "system" fn filter(info: *const EXCEPTION_POINTERS) -> i32 {
	// SAFETY: the system passes the exception's pointers, valid during the call.
	let record = unsafe { info.as_ref().and_then(|info| info.ExceptionRecord.as_ref()) };
	let installed = INSTALLED.get();
	if let Some(record) = record {
		let thread = std::thread::current();
		let text = describe(record.ExceptionCode.0 as u32, &place(record.ExceptionAddress as usize), thread.name().unwrap_or("unnamed"));
		squirrel_bb::html_log::critical(&text);
		eprintln!("bb crashed: {text}");
		crate::failure::show_crash(&text, installed.and_then(|(logs, _)| logs.as_deref()));
	}
	match installed.and_then(|(_, previous)| *previous) {
		// SAFETY: the filter that was installed before, called as the system would.
		Some(previous) => unsafe { previous(info) },
		None => EXCEPTION_CONTINUE_SEARCH,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_crash_says_what_and_where() {
		assert_eq!(
			describe(0xC000_0005, "libcef.dll+0x1a2b", "main"),
			"a system exception 0xc0000005 (an access violation) at libcef.dll+0x1a2b, on the thread 'main'"
		);
		assert_eq!(describe(0x8000_0003, "0x10", "x"), "a system exception 0x80000003 at 0x10, on the thread 'x'");
	}

	/// In a process of its own (this binary run again, with `CRASH_LOG` set): report
	/// crashes to a log, then crash.
	#[test]
	fn crash_helper() {
		let Some(log) = std::env::var_os("BB_TEST_CRASH_LOG") else { return };
		squirrel_bb::html_log::open(std::path::Path::new(&log), "crash").unwrap();
		report_crashes(None);
		// SAFETY: none: the point is an access violation.
		unsafe { std::ptr::write_volatile(std::ptr::null_mut::<u32>(), 1) };
	}

	#[test]
	fn a_native_crash_is_logged() {
		let dir = std::env::temp_dir().join(format!("bb-crash-{}", std::process::id()));
		std::fs::create_dir_all(&dir).unwrap();
		let log = dir.join("log.html");
		let status = std::process::Command::new(std::env::current_exe().unwrap())
			.args(["win_process::crash::tests::crash_helper", "--exact", "--nocapture"])
			.env("BB_TEST_CRASH_LOG", &log)
			.env("BB_ERROR_DIALOG", "0")
			.status()
			.unwrap();
		assert!(!status.success());
		let text = std::fs::read_to_string(&log).unwrap();
		let exe = std::env::current_exe().unwrap();
		let exe = exe.file_name().unwrap().to_string_lossy();
		let line = format!("A critical error occured: a system exception 0xc0000005 (an access violation) at {exe}+0x");
		assert!(text.contains(&line), "{text}");
		std::fs::remove_dir_all(&dir).unwrap();
	}

	#[test]
	fn an_address_names_its_module() {
		let at = place(describe as fn(u32, &str, &str) -> String as usize);
		assert!(at.contains(".exe+0x"), "{at}");
	}
}
