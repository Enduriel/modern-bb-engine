//! What the player is told when bb can't go on: every error that ends its start or its
//! run, and a panic, in a message box ([`requirements::dialog`]) besides stderr and the
//! log, since a player who double-clicked bb sees neither. Not the command line's
//! errors (an unknown argument, a bad value, `--data` or `BB_DATA_DIR` naming no
//! install): a double-click passes none, so they go to stderr only (`main.rs`).
//!
//! - A machine that can't run the game: what it lacks, and the minimum requirements
//!   ([`requirements::explain`]).
//! - Any other error as bb starts: what it was, the game's folder bb was running (its
//!   files may be damaged: Steam can check them), and where the log is.
//! - Any other error once the game runs (playing, or quitting): what it was, and where
//!   the log is. Not the game's files: they loaded. (A script error doesn't end the game
//!   then: it is logged and the game goes on, as the original's, see
//!   `squirrel_bb::ScriptErrors`.)
//! - A panic: what it was and where the log is, as a crash, once. Except one in work
//!   that may fail without ending the game ([`catch`]: a memory snapshot's part): that
//!   is an error of the work's, in words, with no message box or crash line.

use std::cell::{Cell, RefCell};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::requirements;

/// When the error that ends bb came.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum When {
	/// As bb started: finding the game, its files, the window, the scripts' boot (and
	/// the campaign `--new-campaign` starts).
	Starting,
	/// Once the game ran: playing, or quitting.
	Running,
}

/// The message box's title for `error`, which came `when` (a machine that can't run
/// the game can't start it, whenever that shows).
pub fn title(error: &str, when: When) -> &'static str {
	match when {
		When::Running if requirements::explain(error, None).is_none() => "Battle Brothers stopped",
		_ => "Battle Brothers can't start",
	}
}

/// The player's message for `error`, which came `when`, with the game's data folder bb
/// was running if it had found one (as it starts), and the log's folder if there is one.
pub fn message(error: &str, when: When, data_dir: Option<&Path>, logs: Option<&Path>) -> String {
	if let Some(text) = requirements::explain(error, logs) {
		return text;
	}
	let mut text = match when {
		When::Starting => format!("Battle Brothers couldn't start: {error}"),
		When::Running => format!("Battle Brothers stopped on an error: {error}"),
	};
	if let Some(data) = data_dir.filter(|_| when == When::Starting) {
		text.push_str(&format!(
			"\n\nIt was running the game's files in {}. If they're damaged, Steam can check them: the \
			 game's Properties, Installed Files, Verify integrity of game files.",
			data.display()
		));
	}
	if let Some(logs) = logs {
		text.push_str(&format!("\n\nDetails: log.html in {}", logs.display()));
	}
	text
}

/// Show a panic in the message box too, once (a panic on another thread while the
/// first one's box is up adds none). Call once, in the game's own process (not the
/// browser's child processes).
pub fn show_panics(logs: Option<std::path::PathBuf>) {
	let default = std::panic::take_hook();
	std::panic::set_hook(Box::new(move |info| {
		if CATCHING.get() {
			CAUGHT.replace(Some(info.to_string()));
			return;
		}
		default(info);
		show_crash(&info.to_string(), logs.as_deref());
	}));
}

thread_local! {
	/// Whether this thread runs work that may fail without ending the game ([`catch`]).
	static CATCHING: Cell<bool> = const { Cell::new(false) };
	/// The panic that work met, as the hook saw it (with where it came from).
	static CAUGHT: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Run `f`, whose failure needn't end the game: a panic in it is its error, in words,
/// and goes no further (no message box, no crash line in the log, the game goes on).
pub fn catch<R>(f: impl FnOnce() -> R) -> Result<R, String> {
	let outer = CATCHING.replace(true);
	let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
	CATCHING.set(outer);
	result.map_err(|payload| {
		CAUGHT.take().unwrap_or_else(|| {
			let text = payload.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| payload.downcast_ref::<String>().cloned());
			format!("panicked: {}", text.unwrap_or_default())
		})
	})
}

/// Tell the player bb crashed (`what` went wrong; `logs`: the log's folder), in the
/// message box, once in the process: a panic, or a native crash on Windows
/// (`win_process::crash`); a second crash while the first one's box is up adds none.
pub fn show_crash(what: &str, logs: Option<&Path>) {
	static SHOWN: AtomicBool = AtomicBool::new(false);
	if SHOWN.swap(true, Ordering::Relaxed) {
		return;
	}
	let mut text = format!("Battle Brothers crashed: {what}");
	if let Some(logs) = logs {
		text.push_str(&format!("\n\nDetails: log.html in {}", logs.display()));
	}
	requirements::dialog::show("Battle Brothers crashed", &text);
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_caught_panic_is_an_error() {
		assert_eq!(catch(|| 3), Ok(3));
		let error = catch(|| -> u32 { panic!("the part broke") }).unwrap_err();
		assert!(error.contains("the part broke"), "{error}");
	}

	#[test]
	fn any_error_says_what_it_was_and_where_to_look() {
		let text = message("data_003.dat: invalid zip", When::Starting, Some(Path::new("/games/BB/data")), Some(Path::new("/logs")));
		assert!(text.starts_with("Battle Brothers couldn't start: data_003.dat: invalid zip"), "{text}");
		assert!(text.contains("/games/BB/data") && text.contains("Verify integrity"), "{text}");
		assert!(text.ends_with("log.html in /logs"), "{text}");
		assert_eq!(message("unknown argument --x", When::Starting, None, None), "Battle Brothers couldn't start: unknown argument --x");
		assert_eq!(title("data_003.dat: invalid zip", When::Starting), "Battle Brothers can't start");
	}

	#[test]
	fn an_error_once_the_game_runs_is_not_a_start_and_not_its_files() {
		let text = message("the surface was lost", When::Running, Some(Path::new("/games/BB/data")), Some(Path::new("/logs")));
		assert_eq!(text, "Battle Brothers stopped on an error: the surface was lost\n\nDetails: log.html in /logs");
		assert!(!text.contains("start") && !text.contains("Verify integrity"), "{text}");
		assert_eq!(title("the surface was lost", When::Running), "Battle Brothers stopped");
	}

	#[test]
	fn a_hardware_failure_names_the_requirements() {
		let text = message(&requirements::unsupported("no Vulkan"), When::Running, None, None);
		assert!(text.contains(requirements::minimum()), "{text}");
		assert_eq!(title(&requirements::unsupported("no Vulkan"), When::Running), "Battle Brothers can't start");
	}
}
