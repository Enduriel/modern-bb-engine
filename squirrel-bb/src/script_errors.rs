//! What a script error does to the engine's own calls into the scripts: the states'
//! hooks (`onInit`, `onUpdate`, `onRender`, `onFinish`, `onShow`, input, the background
//! task, the siblings' messages) and the UI's replies to the scripts.
//!
//! In the original such an error is logged (a `Script Error` row with its stack) and
//! the game goes on: the call it ended is simply over. Probed 2026-10-22: an error in
//! the world state's `onUpdate` (the tooltip's, on a tile under a cursor not yet placed)
//! and one in its `onFinish` (`quit_with_event_open.rs`) are each logged once; the
//! frames go on, the finished state is gone and the main menu shows. The natives that
//! call script callbacks behave the same: they return normally to their caller (probed,
//! `callback_errors.rs`). The state natives that run hooks (`show`, `hide`, `finish`,
//! `add`, `sendMessageToSiblings`, `finishChildren`) are taken to do so too (not probed
//! for them).

/// What the engine does with a script error raised in a call it makes into the scripts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScriptErrors {
	/// The engine's call fails with it (`Err`), and so does the frame or native it was
	/// part of: a test or tool sees it at once. The default.
	#[default]
	Fail,
	/// As the original's: the error has been reported (the VM's error handler, see
	/// [`report_script_errors`](crate::report_script_errors)) and the game goes on, the
	/// call it ended over. For a game a player plays.
	GoOn,
}

impl ScriptErrors {
	/// `result` of a call into the scripts, as this policy leaves it.
	pub fn settle<T: Default>(self, result: Result<T, String>) -> Result<T, String> {
		match (self, result) {
			(ScriptErrors::GoOn, Err(_)) => Ok(T::default()),
			(_, result) => result,
		}
	}
}
