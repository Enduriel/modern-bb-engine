//! The game's scripts with their UI: [`Front`].
//!
//! - Built plainly, the UI's screens run headless: nothing is drawn over the frame, the
//!   cursor is never over the UI, and the campaign's opening event is closed at the
//!   start, since nothing could click it away.
//! - With the `ui` feature, the real page runs in the browser host (`bb-ui`), is drawn
//!   over the frame, and takes the player's input where the original's did.
//!
//! How input divides between the page and the states follows the vanilla scripts:
//! - Every mouse move reaches the states. They call `Cursor.setPosition` themselves,
//!   and that is how they learn whether the cursor is over the UI.
//! - A button press or the wheel goes to the page where the page isn't transparent,
//!   and to the states elsewhere. The states' click handlers don't check for the UI.
//!   A release follows its press.
//! - Keys reach the page (its dialogs listen on the document), and the states too,
//!   unless the page has a text field focused.

use std::path::Path;
use std::sync::Arc;

use bb_engine::files::GameFiles;
use bb_engine::harness::{self, GameEngine};

#[cfg(not(feature = "ui"))]
mod headless;
#[cfg(feature = "ui")]
mod page;

#[cfg(not(feature = "ui"))]
pub use headless::Front;
#[cfg(feature = "ui")]
pub use page::Front;

/// A key as the window reports it.
#[derive(Clone, Debug, Default)]
pub struct Key {
	/// The engine's code (what the states see), if the scripts can see the key.
	pub engine: Option<i64>,
	/// The Windows virtual-key code (what the page sees), if it has one.
	pub windows: Option<i32>,
	/// What the press typed, if anything.
	pub text: Option<String>,
	pub down: bool,
	/// A held key repeating.
	pub repeat: bool,
}

/// Play the game's sound on the system's output device (kira), reading the sounds
/// from the install at `data_dir`; without a device the game stays silent. Before the
/// menu boots, so its music starts there.
fn play_on_device(engine: &GameEngine, data_dir: &Path) {
	let out = || -> Result<bb_audio::KiraOut<_>, String> {
		let io = |e: std::io::Error| e.to_string();
		let files = GameFiles::open(harness::data_archives(data_dir).map_err(io)?).map_err(io)?;
		// The original sets the rolloff once, from the screen's width at startup.
		let (width, _, _) = engine.video().screen();
		bb_audio::KiraOut::device(Arc::new(files), bb_audio::gains::rolloff_for_width(width as u32))
	};
	match out() {
		Ok(out) => engine.audio().set_out(Box::new(out)),
		Err(e) => eprintln!("no sound: {e}"),
	}
}
