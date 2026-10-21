//! A message box for a start that can't go on, shown with what the system already has
//! (no crate): Windows' `MessageBoxW`; macOS's `osascript`; on Linux the first of
//! `zenity`, `kdialog` and `xmessage` that runs (Steam's runtime and most desktops have
//! one), else nothing beyond stderr and the log.
//!
//! It waits until the player closes it, as the original's error boxes do.
//! `BB_ERROR_DIALOG=0` turns it off (the tests, which can't click it away), and so does
//! having no display to show it on.

/// Show `text` under `title` and wait until it is closed; whether one was shown.
pub fn show(title: &str, text: &str) -> bool {
	if std::env::var_os("BB_ERROR_DIALOG").is_some_and(|v| v == "0") {
		return false;
	}
	show_native(title, text)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn show_native(title: &str, text: &str) -> bool {
	use std::process::{Command, Stdio};
	if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
		return false;
	}
	let tools: [(&str, Vec<String>); 3] = [
		("zenity", vec!["--error".into(), "--no-markup".into(), format!("--title={title}"), format!("--text={text}")]),
		("kdialog", vec!["--title".into(), title.into(), "--error".into(), text.into()]),
		("xmessage", vec!["-center".into(), "-title".into(), title.into(), text.into()]),
	];
	tools.into_iter().any(|(tool, args)| {
		// A tool that started is enough, however it ended (the player closed it).
		Command::new(tool).args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok()
	})
}

#[cfg(target_os = "macos")]
fn show_native(title: &str, text: &str) -> bool {
	// AppleScript string literals: backslashes and quotes escaped.
	let quote = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
	let script = format!("display alert {} message {} as critical", quote(title), quote(text));
	std::process::Command::new("osascript").args(["-e", &script]).status().is_ok()
}

#[cfg(windows)]
fn show_native(title: &str, text: &str) -> bool {
	use std::ffi::c_void;
	/// `MB_OK | MB_ICONERROR`.
	const STYLE: u32 = 0x10;
	#[link(name = "user32")]
	unsafe extern "system" {
		fn MessageBoxW(owner: *mut c_void, text: *const u16, caption: *const u16, style: u32) -> i32;
	}
	let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
	let (text, title) = (wide(text), wide(title));
	// SAFETY: both strings are NUL-terminated UTF-16 that outlive the call; no owner.
	unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), STYLE) != 0 }
}

#[cfg(not(any(unix, windows)))]
fn show_native(_title: &str, _text: &str) -> bool {
	false
}
