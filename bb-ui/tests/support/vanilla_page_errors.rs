//! Page errors that are vanilla's own: its UI throws them in the original too, under the
//! same order of the game's calls and the page's tasks, and the original logs them and
//! carries on (an uncaught exception is a `JS …` error row in its log.html), as ours
//! does. A test that fails on page errors leaves these out, but only with the stack
//! that shows it is that one place in vanilla's code.

/// One of vanilla's own errors: the message the page logs (Chromium's wording), and a
/// line its stack must show.
pub struct VanillaError {
	pub message: &'static str,
	pub stack: &'static str,
	/// What it is, and the probe that showed the original throwing it.
	pub why: &'static str,
}

/// The turn bar's first slot: a unit selected for the slot anew (a dying unit's slot
/// removed and the next one added arms the slot's 10 ms timer again, which selects the
/// first unit again) whose slot goes (its turn ended) within about three frames: the
/// slot's slide-out (30 ms for units hidden from the player) ends before the image's
/// resize (180 ms), whose `complete` then reads the removed image's placeholder.
/// `bb-ui/probes/turnbar_race.{nut,js}` in the original (2026-10-24): "TypeError:
/// undefined is not an object (evaluating 'entityImage.data('placeholder').css')",
/// logged, the page going on, for removals two and three frames after.
pub const TURN_BAR_FIRST_SLOT: VanillaError = VanillaError {
	message: "Uncaught TypeError: Cannot read properties of undefined (reading 'css')",
	stack: "turnsequencebar_module.js:1011:",
	why: "the turn bar's first-slot race (turnsequencebar_module.js:1011), which the original throws too",
};

pub const ALL: [VanillaError; 1] = [TURN_BAR_FIRST_SLOT];

/// Which of the page's error messages are vanilla's own errors: for each, what it is, or
/// `None` for a problem. `stacks` are the page's uncaught errors' stacks: a message
/// counts as vanilla's own only while as many stacks show its line as messages are taken
/// for it.
pub fn own(messages: &[&str], stacks: &[String]) -> Vec<Option<&'static str>> {
	let mut left: Vec<usize> = ALL.iter().map(|e| stacks.iter().filter(|s| s.contains(e.stack)).count()).collect();
	messages
		.iter()
		.map(|message| match ALL.iter().position(|e| message.starts_with(e.message)) {
			Some(i) if left[i] > 0 => {
				left[i] -= 1;
				Some(ALL[i].why)
			}
			_ => None,
		})
		.collect()
}
