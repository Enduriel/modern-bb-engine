//! The editing keys on macOS: copy, cut, paste, select all, undo and redo.
//!
//! On Windows and Linux, Chromium's renderer runs these itself on Ctrl+C and the rest,
//! once the page's keydown handlers have left the event alone. On macOS it doesn't:
//! there they are the application menu's Cmd shortcuts, which an offscreen page has
//! none of. So on macOS the page's keys come back here once the page has had them
//! ([`KeyboardHandler::on_key_event`] gets only the events the page didn't cancel and
//! Chromium didn't act on), and an editing key runs its command on the focused frame,
//! as the menu would. Both Cmd (the Mac's own) and Ctrl (what mods written on
//! Windows check, `event.ctrlKey`) work. The page's keydown and
//! keyup stay as they were sent: `ctrlKey` or `metaKey` as held.

use cef::*;

/// An editing command a key runs on the focused frame.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EditCommand {
	Copy,
	Cut,
	Paste,
	SelectAll,
	Undo,
	Redo,
}

/// `EVENTFLAG_SHIFT_DOWN`, `_CONTROL_DOWN`, `_ALT_DOWN`, `_COMMAND_DOWN` (`cef_types.h`).
const SHIFT: u32 = 1 << 1;
const CONTROL: u32 = 1 << 2;
const ALT: u32 = 1 << 3;
const COMMAND: u32 = 1 << 7;

/// The command a key going down runs: `code` is its Windows virtual-key code, and
/// `modifiers` the `EVENTFLAG_*` bits held. Cmd or Ctrl, not both and not Alt; Shift
/// only for redo (Shift+Z), and Y redoes too, as on Windows.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(super) fn edit_command(code: i32, modifiers: u32) -> Option<EditCommand> {
	let command = modifiers & (CONTROL | COMMAND);
	if command != CONTROL && command != COMMAND || modifiers & ALT != 0 {
		return None;
	}
	let shift = modifiers & SHIFT != 0;
	Some(match (u8::try_from(code).ok()?, shift) {
		(b'C', false) => EditCommand::Copy,
		(b'X', false) => EditCommand::Cut,
		(b'V', false) => EditCommand::Paste,
		(b'A', false) => EditCommand::SelectAll,
		(b'Z', false) => EditCommand::Undo,
		(b'Z', true) | (b'Y', false) => EditCommand::Redo,
		_ => return None,
	})
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
impl EditCommand {
	/// Run it on `frame`.
	fn run(self, frame: &Frame) {
		match self {
			EditCommand::Copy => frame.copy(),
			EditCommand::Cut => frame.cut(),
			EditCommand::Paste => frame.paste(),
			EditCommand::SelectAll => frame.select_all(),
			EditCommand::Undo => frame.undo(),
			EditCommand::Redo => frame.redo(),
		}
	}
}

/// What to do with a key the page left alone: run its editing command on the focused
/// frame, if it has one. Whether it did.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn unhandled_key(browser: Option<&mut Browser>, event: Option<&KeyEvent>) -> bool {
	let Some(event) = event else { return false };
	if event.type_ != KeyEventType::RAWKEYDOWN && event.type_ != KeyEventType::KEYDOWN {
		return false;
	}
	let Some(command) = edit_command(event.windows_key_code, event.modifiers) else { return false };
	let Some(frame) = browser.and_then(|b| b.focused_frame()) else { return false };
	command.run(&frame);
	true
}

#[cfg(target_os = "macos")]
wrap_keyboard_handler! {
	pub(super) struct EditKeys;

	impl KeyboardHandler {
		fn on_key_event(
			&self,
			browser: Option<&mut Browser>,
			event: Option<&KeyEvent>,
			_os_event: *mut u8,
		) -> ::std::os::raw::c_int {
			i32::from(unhandled_key(browser, event))
		}
	}
}

/// The page's keyboard handler: the editing keys on macOS, none elsewhere (Chromium's
/// renderer has them there).
pub(super) fn keyboard_handler() -> Option<KeyboardHandler> {
	#[cfg(target_os = "macos")]
	return Some(EditKeys::new());
	#[cfg(not(target_os = "macos"))]
	None
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn cmd_or_ctrl_with_the_letter() {
		for held in [CONTROL, COMMAND] {
			assert_eq!(edit_command(0x43, held), Some(EditCommand::Copy));
			assert_eq!(edit_command(0x58, held), Some(EditCommand::Cut));
			assert_eq!(edit_command(0x56, held), Some(EditCommand::Paste));
			assert_eq!(edit_command(0x41, held), Some(EditCommand::SelectAll));
			assert_eq!(edit_command(0x5A, held), Some(EditCommand::Undo));
			assert_eq!(edit_command(0x5A, held | SHIFT), Some(EditCommand::Redo));
			assert_eq!(edit_command(0x59, held), Some(EditCommand::Redo));
		}
	}

	#[test]
	fn other_combinations_are_left_alone() {
		assert_eq!(edit_command(0x43, 0), None);
		assert_eq!(edit_command(0x43, SHIFT), None);
		assert_eq!(edit_command(0x43, CONTROL | COMMAND), None);
		assert_eq!(edit_command(0x43, CONTROL | ALT), None);
		assert_eq!(edit_command(0x56, COMMAND | SHIFT), None);
		assert_eq!(edit_command(0x44, CONTROL), None);
		assert_eq!(edit_command(-1, CONTROL), None);
	}
}
