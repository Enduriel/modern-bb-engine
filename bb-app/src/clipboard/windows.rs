//! The clipboard through Windows' clipboard functions (user32), as UTF-16 text
//! (`CF_UNICODETEXT`).

use std::ffi::c_void;
use std::time::Duration;

/// `CF_UNICODETEXT`: NUL-terminated UTF-16.
const CF_UNICODETEXT: u32 = 13;
/// `GMEM_MOVEABLE`: what the clipboard takes ownership of.
const GMEM_MOVEABLE: u32 = 0x0002;
/// How often to try opening the clipboard while another program holds it open.
const OPEN_TRIES: u32 = 10;

#[link(name = "user32")]
unsafe extern "system" {
	fn OpenClipboard(owner: *mut c_void) -> i32;
	fn CloseClipboard() -> i32;
	fn EmptyClipboard() -> i32;
	fn GetClipboardData(format: u32) -> *mut c_void;
	fn SetClipboardData(format: u32, memory: *mut c_void) -> *mut c_void;
}

#[link(name = "kernel32")]
unsafe extern "system" {
	fn GlobalAlloc(flags: u32, bytes: usize) -> *mut c_void;
	fn GlobalFree(memory: *mut c_void) -> *mut c_void;
	fn GlobalLock(memory: *mut c_void) -> *mut c_void;
	fn GlobalUnlock(memory: *mut c_void) -> i32;
	fn GlobalSize(memory: *mut c_void) -> usize;
}

/// The clipboard, open until dropped.
struct Open;

impl Open {
	/// Open the clipboard for this thread, waiting a little while another program has
	/// it open.
	fn new() -> Option<Open> {
		for _ in 0..OPEN_TRIES {
			// SAFETY: no owner window; the clipboard stays open until `drop`.
			if unsafe { OpenClipboard(std::ptr::null_mut()) } != 0 {
				return Some(Open);
			}
			std::thread::sleep(Duration::from_millis(10));
		}
		None
	}
}

impl Drop for Open {
	fn drop(&mut self) {
		// SAFETY: opened by `Open::new` on this thread.
		unsafe { CloseClipboard() };
	}
}

/// The clipboard's text; `Ok(None)` for none, `Err` if it couldn't be opened.
pub(super) fn read() -> Result<Option<String>, ()> {
	let _open = Open::new().ok_or(())?;
	// SAFETY: the clipboard is open; the handle stays the clipboard's (not freed here)
	// and is read only between its lock and unlock, within its size.
	unsafe {
		let memory = GetClipboardData(CF_UNICODETEXT);
		if memory.is_null() {
			return Ok(None);
		}
		let units = GlobalSize(memory) / 2;
		let data = GlobalLock(memory) as *const u16;
		if data.is_null() {
			return Err(());
		}
		let all = std::slice::from_raw_parts(data, units);
		let length = all.iter().position(|&u| u == 0).unwrap_or(all.len());
		let text = String::from_utf16_lossy(&all[..length]);
		GlobalUnlock(memory);
		Ok(Some(text).filter(|t| !t.is_empty()))
	}
}

/// Put `text` on the clipboard.
pub(super) fn write(text: &str) -> Result<(), ()> {
	let units: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
	// SAFETY: the block is `units.len()` u16s, written only between its lock and unlock;
	// once `SetClipboardData` takes it, the clipboard owns it (freed here otherwise).
	unsafe {
		let memory = GlobalAlloc(GMEM_MOVEABLE, units.len() * 2);
		if memory.is_null() {
			return Err(());
		}
		let target = GlobalLock(memory) as *mut u16;
		if target.is_null() {
			GlobalFree(memory);
			return Err(());
		}
		std::ptr::copy_nonoverlapping(units.as_ptr(), target, units.len());
		GlobalUnlock(memory);
		let Some(_open) = Open::new() else {
			GlobalFree(memory);
			return Err(());
		};
		EmptyClipboard();
		if SetClipboardData(CF_UNICODETEXT, memory).is_null() {
			GlobalFree(memory);
			return Err(());
		}
	}
	Ok(())
}
