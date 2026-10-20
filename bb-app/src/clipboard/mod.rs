//! The system's clipboard, which the page's copy and paste use (`bb_ui::clipboard`):
//! text copied in the game reaches the player's other programs, and text copied there
//! can be pasted into the game, as in the original (Coherent on Windows).
//!
//! Each system's clipboard is reached as it offers it, without a library:
//! - Windows: its clipboard functions ([`windows`]).
//! - macOS: `pbcopy` and `pbpaste` ([`command`]).
//! - Linux and the BSDs: the display's clipboard programs ([`command`]): `wl-copy` and
//!   `wl-paste` (wl-clipboard) on Wayland, `xclip` or `xsel` on X11 (and on Wayland
//!   when wl-clipboard isn't installed: XWayland's clipboard, which some compositors
//!   share with Wayland's). The game's window can't serve the selection itself without
//!   a Wayland or X11 library, and the browser's own clipboard can't reach Wayland's
//!   from an offscreen page (the compositor only lets the focused window, the game's,
//!   take or read the selection).
//!
//! What the game copies is kept too: when the system's clipboard can't be reached (no
//! clipboard program), a paste gets the game's last copy.

#[cfg(unix)]
mod command;
#[cfg(windows)]
mod windows;

use std::sync::Mutex;

/// The system's clipboard, with the game's last copy to fall back on.
pub struct SystemClipboard {
	/// What the game copied last.
	own: Mutex<Option<String>>,
	/// How this system's clipboard is reached, if it can be.
	system: Option<System>,
}

impl SystemClipboard {
	pub fn new() -> SystemClipboard {
		SystemClipboard { own: Mutex::new(None), system: System::find() }
	}

	/// The text on the system's clipboard (`None` if there is none), or the game's last
	/// copy when the system's can't be reached.
	pub fn read_text(&self) -> Option<String> {
		match self.system.as_ref().map(System::read) {
			Some(Ok(text)) => text.filter(|t| !t.is_empty()),
			Some(Err(())) | None => self.own.lock().unwrap_or_else(|e| e.into_inner()).clone(),
		}
	}

	/// Put `text` on the system's clipboard, and keep it.
	pub fn write_text(&self, text: &str) {
		*self.own.lock().unwrap_or_else(|e| e.into_inner()) = Some(text.to_string());
		if let Some(system) = &self.system {
			if system.write(text).is_err() {
				eprintln!("clipboard: the system's clipboard didn't take the copy ({})", system.name());
			}
		}
	}
}

impl Default for SystemClipboard {
	fn default() -> Self {
		SystemClipboard::new()
	}
}

#[cfg(feature = "ui")]
impl bb_ui::clipboard::Clipboard for SystemClipboard {
	fn read_text(&self) -> Option<String> {
		SystemClipboard::read_text(self)
	}

	fn write_text(&self, text: &str) {
		SystemClipboard::write_text(self, text);
	}
}

/// How the system's clipboard is reached.
enum System {
	#[cfg(unix)]
	Programs(command::Programs),
	#[cfg(windows)]
	Windows,
}

impl System {
	#[cfg(unix)]
	fn find() -> Option<System> {
		command::Programs::find().map(System::Programs)
	}

	#[cfg(windows)]
	fn find() -> Option<System> {
		Some(System::Windows)
	}

	/// The clipboard's text (`None` if it holds none); `Err` if it can't be reached.
	fn read(&self) -> Result<Option<String>, ()> {
		match self {
			#[cfg(unix)]
			System::Programs(p) => p.read(),
			#[cfg(windows)]
			System::Windows => windows::read(),
		}
	}

	fn write(&self, text: &str) -> Result<(), ()> {
		match self {
			#[cfg(unix)]
			System::Programs(p) => p.write(text),
			#[cfg(windows)]
			System::Windows => windows::write(text),
		}
	}

	fn name(&self) -> &'static str {
		match self {
			#[cfg(unix)]
			System::Programs(p) => p.name(),
			#[cfg(windows)]
			System::Windows => "Windows",
		}
	}
}
