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
