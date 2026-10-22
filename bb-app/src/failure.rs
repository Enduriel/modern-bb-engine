//! What the player is told when bb can't go on: every error that ends its start or its
//! run, and a panic, in a message box ([`requirements::dialog`]) besides stderr and the
//! log, since a player who double-clicked bb sees neither.
//!
//! - A machine that can't run the game: what it lacks, and the minimum requirements
//!   ([`requirements::explain`]).
//! - Any other error: what it was, the game's folder bb was running (its files may be
//!   damaged: Steam can check them), and where the log is.
//! - A panic: the same, as a crash, once.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::requirements;

/// The message box's title.
pub const TITLE: &str = "Battle Brothers can't start";

/// The player's message for `error`, with the game's data folder bb was running if it
/// had found one, and the log's folder if there is one.
pub fn message(error: &str, data_dir: Option<&Path>, logs: Option<&Path>) -> String {
	if let Some(text) = requirements::explain(error, logs) {
		return text;
	}
	let mut text = format!("Battle Brothers couldn't start: {error}");
	if let Some(data) = data_dir {
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
	static SHOWN: AtomicBool = AtomicBool::new(false);
	let default = std::panic::take_hook();
	std::panic::set_hook(Box::new(move |info| {
		default(info);
		if SHOWN.swap(true, Ordering::Relaxed) {
			return;
		}
		let mut text = format!("Battle Brothers crashed: {info}");
		if let Some(logs) = &logs {
			text.push_str(&format!("\n\nDetails: log.html in {}", logs.display()));
		}
		requirements::dialog::show("Battle Brothers crashed", &text);
	}));
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn any_error_says_what_it_was_and_where_to_look() {
		let text = message("data_003.dat: invalid zip", Some(Path::new("/games/BB/data")), Some(Path::new("/logs")));
		assert!(text.starts_with("Battle Brothers couldn't start: data_003.dat: invalid zip"), "{text}");
		assert!(text.contains("/games/BB/data") && text.contains("Verify integrity"), "{text}");
		assert!(text.ends_with("log.html in /logs"), "{text}");
		assert_eq!(message("unknown argument --x", None, None), "Battle Brothers couldn't start: unknown argument --x");
	}

	#[test]
	fn a_hardware_failure_names_the_requirements() {
		let text = message(&requirements::unsupported("no Vulkan"), None, None);
		assert!(text.contains(requirements::minimum()), "{text}");
	}
}
