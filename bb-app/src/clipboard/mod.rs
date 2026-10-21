//! The system's clipboard, which the page's copy and paste use (`bb_ui::clipboard`):
//! text copied in the game reaches the player's other programs, and text copied there
//! can be pasted into the game, as in the original (Coherent on Windows).
//!
//! Each system's clipboard is reached as it offers it:
//! - Windows: its clipboard functions ([`windows`]).
//! - macOS: `pbcopy` and `pbpaste` ([`command`]).
//! - Linux and the BSDs, the game's window on Wayland: the window's own connection to
//!   the compositor ([`wayland`], smithay-clipboard). The browser's own clipboard can't
//!   reach Wayland's from an offscreen page: the compositor only lets the focused
//!   window, the game's, take or read the selection.
//! - Linux and the BSDs otherwise (on X11, without a window, or when the compositor has
//!   no clipboard, as gamescope): the display's clipboard programs ([`command`]):
//!   `wl-copy` and `wl-paste` (wl-clipboard) on Wayland, `xclip` or `xsel` on X11 (and on
//!   Wayland when wl-clipboard isn't installed: XWayland's clipboard, which some
//!   compositors share with Wayland's). The game's window doesn't serve X11's
//!   selection itself: that would take an X11 library.
//!
//! What the game copies is kept too: when the system's clipboard can't be reached (no
//! clipboard program), a paste gets the game's last copy. With one, a clipboard
//! holding no text (a picture: the program finds no text to give) pastes nothing, as
//! the original's does (probed in the original: an image on the clipboard, Ctrl+V's paste
//! event carried ""), not the game's earlier copy.

#[cfg(unix)]
mod command;
#[cfg(all(unix, not(target_os = "macos")))]
mod wayland;
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
	/// The clipboard without a window: the system's programs (see the module).
	pub fn new() -> SystemClipboard {
		SystemClipboard { own: Mutex::new(None), system: System::find(None) }
	}

	/// The clipboard for the game's window on `display`: on Wayland, served through the
	/// window's own connection.
	pub fn for_window(display: &winit::event_loop::OwnedDisplayHandle) -> SystemClipboard {
		let clipboard = SystemClipboard { own: Mutex::new(None), system: System::find(Some(display)) };
		eprintln!("clipboard: {}", clipboard.system_name().unwrap_or("the game's own (no system clipboard)"));
		clipboard
	}

	/// How the system's clipboard is reached, if it is.
	pub fn system_name(&self) -> Option<&'static str> {
		self.system.as_ref().map(System::name)
	}

	/// The text on the system's clipboard (`None` if there is none), or the game's last
	/// copy when there is no system clipboard to reach.
	pub fn read_text(&self) -> Option<String> {
		pasted(self.system.as_ref().map(System::read), || self.own.lock().unwrap_or_else(|e| e.into_inner()).clone())
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

/// What a paste gets: what the system's clipboard gave (`Err`: no text it could give),
/// or with no system clipboard (`None`) the game's own last copy.
fn pasted(system: Option<Result<Option<String>, ()>>, own: impl FnOnce() -> Option<String>) -> Option<String> {
	match system {
		Some(read) => read.ok().flatten().filter(|t| !t.is_empty()),
		None => own(),
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
	#[cfg(all(unix, not(target_os = "macos")))]
	Wayland(wayland::Wayland),
	#[cfg(windows)]
	Windows,
}

impl System {
	/// The game window's display's clipboard if it serves one (Wayland), else the
	/// programs for the display.
	#[cfg(unix)]
	fn find(display: Option<&winit::event_loop::OwnedDisplayHandle>) -> Option<System> {
		#[cfg(all(unix, not(target_os = "macos")))]
		if let Some(wayland) = display.and_then(wayland::Wayland::start) {
			return Some(System::Wayland(wayland));
		}
		let _ = display;
		command::Programs::find().map(System::Programs)
	}

	#[cfg(windows)]
	fn find(_: Option<&winit::event_loop::OwnedDisplayHandle>) -> Option<System> {
		Some(System::Windows)
	}

	/// The clipboard's text (`None` if it holds none); `Err` if it can't be reached.
	fn read(&self) -> Result<Option<String>, ()> {
		match self {
			#[cfg(unix)]
			System::Programs(p) => p.read(),
			#[cfg(all(unix, not(target_os = "macos")))]
			System::Wayland(w) => w.read(),
			#[cfg(windows)]
			System::Windows => windows::read(),
		}
	}

	fn write(&self, text: &str) -> Result<(), ()> {
		match self {
			#[cfg(unix)]
			System::Programs(p) => p.write(text),
			#[cfg(all(unix, not(target_os = "macos")))]
			System::Wayland(w) => w.write(text),
			#[cfg(windows)]
			System::Windows => windows::write(text),
		}
	}

	fn name(&self) -> &'static str {
		match self {
			#[cfg(unix)]
			System::Programs(p) => p.name(),
			#[cfg(all(unix, not(target_os = "macos")))]
			System::Wayland(w) => w.name(),
			#[cfg(windows)]
			System::Windows => "Windows",
		}
	}
}

#[cfg(test)]
mod tests {
	use super::pasted;

	/// With an image on the system's clipboard, its program finds no text and fails:
	/// that pasted the game's earlier copy.
	#[test]
	fn the_games_own_copy_stands_in_only_without_a_system_clipboard() {
		let own = || Some("earlier copy".to_string());
		assert_eq!(pasted(Some(Ok(Some("theirs".into()))), own), Some("theirs".into()));
		assert_eq!(pasted(Some(Err(())), own), None, "a clipboard without text");
		assert_eq!(pasted(Some(Ok(None)), own), None);
		assert_eq!(pasted(None, own), Some("earlier copy".into()), "no clipboard program");
	}
}
