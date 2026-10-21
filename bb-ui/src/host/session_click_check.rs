//! Clicks checked for what they do: the page seeing a click only says the browser
//! dispatched it, not that the button's handler ran or did what the player wanted (the
//! click can land on something else, the button can be disabled, the handler can bail).
//! So a caller names what the click should bring about, a condition on the page, and
//! the click is checked against it: clicked again once if it didn't take, and failing
//! with what the clicks hit if it never does.

use std::time::{Duration, Instant};

use super::UiSession;
use crate::json::Json;

/// Installs (once) the page's record of the mouse's button events, and empties it: each
/// `mousedown`, `mouseup` and `click` the window sees, with its target and point.
const TRAIL_START: &str = r#"(function () {
	if (!window.__bbClickTrail) {
		window.__bbDescribe = function (el) {
			if (!el || !el.tagName) return String(el);
			var text = (el.textContent || '').trim().replace(/\s+/g, ' ').slice(0, 30);
			return el.tagName.toLowerCase() + (el.id ? '#' + el.id : '')
				+ (typeof el.className === 'string' && el.className ? '.' + el.className.trim().split(/\s+/).join('.') : '')
				+ (text ? ' "' + text + '"' : '');
		};
		['mousedown', 'mouseup', 'click'].forEach(function (type) {
			window.addEventListener(type, function (e) {
				window.__bbClickTrail.push(type + ' on ' + window.__bbDescribe(e.target) + ' at ' + e.clientX + ',' + e.clientY);
			}, true);
		});
	}
	window.__bbClickTrail = [];
})()"#;

/// How long the game runs for a click to take before it's tried again.
const TAKES: Duration = Duration::from_secs(10);

impl UiSession {
	/// Start recording what the mouse's buttons hit (see [`UiSession::click_trail`]).
	pub(super) fn start_click_trail(&self) -> Result<(), String> {
		self.eval(TRAIL_START).map(|_| ())
	}

	/// Whether the page's JS expression `condition` holds now.
	pub fn holds(&self, condition: &str) -> Result<bool, String> {
		Ok(self.eval(&format!("!!({condition})"))? == Json::Bool(true))
	}

	/// What the mouse's button events since the last click hit, and what's at `(x, y)`
	/// now: for a click that didn't do what it should.
	pub fn click_trail(&self, x: i32, y: i32) -> String {
		let code = format!(
			"(function () {{
	var trail = window.__bbClickTrail || [];
	var at = window.__bbDescribe ? window.__bbDescribe(document.elementFromPoint({x}, {y})) : '?';
	return (trail.length ? trail.join('; ') : 'no button events') + '; now at {x},{y}: ' + at;
}})()"
		);
		match self.eval(&code) {
			Ok(Json::Str(s)) => s,
			Ok(other) => other.to_text(),
			Err(e) => format!("(the page couldn't say: {e})"),
		}
	}

	/// Click `target` (see [`UiSession::locate_when_ready`]) once it's ready, and run the
	/// game until the page's JS expression `done` holds: what the click brings about (a
	/// panel shown, a screen gone). A click that hasn't after [`TAKES`] is logged with
	/// what it hit and made again, once; fails if that one doesn't take either.
	pub fn click_until(&self, target: &str, done: &str, timeout: Duration) -> Result<(), String> {
		let mut first = None;
		loop {
			let (x, y) = self.locate_when_ready(target, timeout).map_err(|e| match &first {
				Some(trail) => format!("clicking {target:?} didn't take ({done} still false; {trail}), and clicking it again: {e}"),
				None => e,
			})?;
			self.click_at(x, y)?;
			let deadline = Instant::now() + TAKES;
			while !self.holds(done)? && Instant::now() < deadline {
				self.run_frames(0.1)?;
			}
			if self.holds(done)? {
				return Ok(());
			}
			let trail = self.click_trail(x, y);
			let Some(first) = &first else {
				let line = format!("clicking {target:?} didn't take ({done} still false after {} s): {trail}; clicking again", TAKES.as_secs());
				eprintln!("{line}");
				self.record("input", line);
				first = Some(trail);
				continue;
			};
			return Err(format!("clicking {target:?} didn't take, twice ({done} still false); first: {first}; again: {trail}"));
		}
	}
}
