//! The clipboard the page's copy and paste use.
//!
//! The original's page (Coherent GT on Windows) copies to and pastes from the system's
//! clipboard: mods rely on it (Plan Perks' Export and Paste buttons call
//! `document.execCommand('copy')` and `('paste')`; text is pasted into the dev
//! console and editors with Ctrl+V). Our browser can't always reach the system's
//! clipboard itself: offscreen it has no window, so on a headless display it keeps a
//! clipboard of its own, and on Wayland the compositor only lets the focused window,
//! the game's, set or read the selection. So the page hands every copy and cut to the
//! game, and asks the game for the text to paste (`host/bootstrap.js`); the game
//! answers from its [`Clipboard`] (the windowed game's: the system's, `bb-app`).

use std::sync::Mutex;

/// Where the page's copied text goes and pasted text comes from. Called from the
/// bridge's thread while the page waits, so it must answer quickly.
pub trait Clipboard: Send + Sync {
	/// The text on the clipboard, if there is any.
	fn read_text(&self) -> Option<String>;
	/// Put `text` on the clipboard.
	fn write_text(&self, text: &str);
}

/// A clipboard of the game's own, shared by nothing else: what the page copied, for it
/// to paste (tests, tools, and the fallback when the system's can't be reached).
#[derive(Default)]
pub struct MemoryClipboard(Mutex<Option<String>>);

impl Clipboard for MemoryClipboard {
	fn read_text(&self) -> Option<String> {
		self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
	}

	fn write_text(&self, text: &str) {
		*self.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(text.to_string());
	}
}

#[cfg(test)]
mod tests {
	use super::{Clipboard, MemoryClipboard};

	#[test]
	fn the_memory_clipboard_gives_back_what_was_copied() {
		let clipboard = MemoryClipboard::default();
		assert_eq!(clipboard.read_text(), None);
		clipboard.write_text("Ünïcode, \"quotes\"\nand lines");
		assert_eq!(clipboard.read_text().as_deref(), Some("Ünïcode, \"quotes\"\nand lines"));
	}
}
