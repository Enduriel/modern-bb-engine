//! A player's input to a [`UiSession`]: pointing at things, clicking, hovering,
//! typing, the wheel.
//!
//! Input reaches the page asynchronously, on another channel than the calls the
//! session makes into it. So each action waits until the page has seen its events
//! before returning: otherwise a check that follows (is the page animating? what's
//! selected?) can run before the input, and see the page as it was.

use std::time::{Duration, Instant};

use super::{Key, UiHost, UiSession, targets};
use crate::json::Json;

/// How long the page may take to see input.
const INPUT_TIMEOUT: Duration = Duration::from_secs(5);

/// How long a game frame waits for the page to see its keys.
const KEYS_TIMEOUT: Duration = Duration::from_millis(250);

/// How long [`UiSession::fill_field`] waits for its field to show up.
const FIELD_TIMEOUT: Duration = Duration::from_secs(30);

impl UiSession {
	/// One game frame of `ms` (the page parked), its keys to the page before the states,
	/// as the original's UI listener got them from its key queue (see
	/// [`squirrel_bb::QueuedInput`]).
	pub(super) fn game_frame(&self, ms: f64) -> Result<(), String> {
		let keys = self.engine().begin_frame(ms);
		if !keys.is_empty() {
			self.keys_to_page(&keys);
		}
		let finished = self.engine().finish_frame(ms, &keys);
		self.host().frame_ran();
		finished
	}

	/// Hand the page a frame's key events and let it handle them before the states get
	/// them: each key's press (again every frame while it's held) and release by
	/// Coherent's code for it, and what keys typed. The page runs until it has seen
	/// every keydown and keyup (its handlers' calls into the scripts answered as they
	/// come, as in the original, where the UI's listener ran first), then is parked
	/// again. What's typed comes after its key; a keypress isn't waited for (a keydown
	/// the page cancels has none).
	///
	/// How many it has seen comes with each park (`page_work.js`), so a held key costs
	/// the page's turn and nothing more: no call into the page to count them (a page
	/// that doesn't say, still loading, is asked).
	fn keys_to_page(&self, keys: &[squirrel_bb::QueuedInput]) {
		let seen = || match self.host().keys_seen() {
			Some(seen) => Some(seen),
			None => self.eval("window.__bbKeysSeen").ok().and_then(|n| n.as_i64()),
		};
		let Some(before) = seen() else { return };
		let host = self.host();
		let mut expected = before;
		for input in keys {
			match *input {
				squirrel_bb::QueuedInput::Key(key) => {
					let code = super::coherent_key_code(key.key);
					host.key(code, key.state != 0, super::modifiers_of_mask(key.modifier));
					expected += 1;
				}
				squirrel_bb::QueuedInput::Text(c, modifier) => {
					host.character(c, super::modifiers_of_mask(modifier));
				}
			}
		}
		let deadline = Instant::now() + KEYS_TIMEOUT;
		// The page has usually seen them by the time it parks again; if not, it gets
		// a while longer each round.
		let mut first = true;
		loop {
			self.host().unpark_for_input();
			for event in self.host().poll() {
				self.handle(event);
			}
			if !std::mem::take(&mut first) {
				if let Some(event) = self.host().next_event(Duration::from_millis(1)) {
					self.handle(event);
				}
			}
			self.park();
			let seen = seen().unwrap_or(before);
			if seen >= expected {
				return;
			}
			if Instant::now() >= deadline {
				self.record("bridge", format!("the page saw {} of a frame's {} key events", seen - before, expected - before));
				return;
			}
		}
	}

	/// Send input with `send`, which returns how many `event`s (a DOM event type) the
	/// page will see from it, and pump until the page has seen them.
	fn input(&self, event: &str, send: impl FnOnce(&UiHost) -> usize) -> Result<(), String> {
		let event_json = Json::Str(event.to_string()).to_text();
		let before = self
			.eval(&format!(
				"(function (type) {{
	var seen = window.__bbSeen || (window.__bbSeen = {{}});
	if (!(type in seen)) {{
		seen[type] = 0;
		window.addEventListener(type, function () {{ seen[type]++; }}, true);
	}}
	return seen[type];
}})({event_json})"
			))?
			.as_i64()
			.unwrap_or(0);
		let expected = before + send(self.host()) as i64;
		let count = format!("window.__bbSeen[{event_json}]");
		let seen = || {
			self.eval(&count)
				.ok()
				.and_then(|n| n.as_i64())
				.is_some_and(|n| n >= expected)
		};
		if self.pump_until(INPUT_TIMEOUT, |_| seen()) {
			Ok(())
		} else {
			Err(format!("the page didn't get the input ({event})"))
		}
	}

	/// Left-click at `(x, y)`, as a player would; returns once the page has seen the
	/// `click` (what buttons listen for), which says the browser dispatched it, not what
	/// it hit or did: [`UiSession::click_until`] checks that, and
	/// [`UiSession::click_trail`] says what it hit.
	pub fn click_at(&self, x: i32, y: i32) -> Result<(), String> {
		self.start_click_trail()?;
		self.input("click", |host| {
			host.click(x, y);
			1
		})
	}

	/// Double-click at `(x, y)`, as a player would: two clicks, which the page sees as
	/// a `dblclick` too.
	pub fn double_click_at(&self, x: i32, y: i32) -> Result<(), String> {
		self.input("dblclick", |host| {
			host.double_click(x, y);
			1
		})
	}

	/// Move the mouse to `(x, y)` and leave it there.
	pub fn move_to(&self, x: i32, y: i32) -> Result<(), String> {
		// The page sees no move if the mouse is there already.
		if self.host().pointer() == Some((x, y)) {
			return Ok(());
		}
		self.input("mousemove", |host| {
			host.mouse_move(x, y);
			1
		})
	}

	/// Move the mouse out of the view, ending any hover (and its tooltip), as if the
	/// player had no mouse over the page.
	pub fn move_away(&self) -> Result<(), String> {
		if self.host().pointer().is_none() {
			return Ok(());
		}
		self.input("mouseout", |host| {
			host.mouse_leave();
			1
		})
	}

	/// Run `find` (JS evaluating to `[x, y]` or `null`): the point it found. `what`
	/// names the target in the error.
	fn locate(&self, find: &str, what: &str) -> Result<(i32, i32), String> {
		let target = self.eval(find)?;
		let xy: Option<Vec<i64>> = target
			.as_array()
			.map(|xy| xy.iter().filter_map(Json::as_i64).collect());
		let Some([x, y]) = xy.and_then(|xy| <[i64; 2]>::try_from(xy).ok()) else {
			return Err(format!("no visible {what}"));
		};
		Ok((x as i32, y as i32))
	}

	/// Click the `.ui-control` (button) whose text is `label`, at its centre.
	pub fn click_button(&self, label: &str) -> Result<(), String> {
		let (x, y) = self.locate(
			&targets::button(label),
			&format!("button labelled {label:?}"),
		)?;
		self.click_at(x, y)
	}

	/// Click the innermost element whose text is `text` (a list entry, a label, a
	/// tab…), at its centre.
	pub fn click_text(&self, text: &str) -> Result<(), String> {
		let (x, y) = self.locate(
			&targets::text(text),
			&format!("element with the text {text:?}"),
		)?;
		self.click_at(x, y)
	}

	/// Click the first element matching the CSS `selector` (e.g. an image button:
	/// `img[src$="icon_end_turn.png"]`), at its centre.
	pub fn click_selector(&self, selector: &str) -> Result<(), String> {
		let (x, y) = self.locate(
			&targets::selector(selector),
			&format!("element matching {selector:?}"),
		)?;
		self.click_at(x, y)
	}

	/// The centre of `target` once the player can see it and the page has stopped
	/// animating, running the game meanwhile, as a player waits for a screen before
	/// pointing at it. `target` is `css:SELECTOR`, or else a button's label or any
	/// element's text. Fails after `timeout`.
	pub fn locate_when_ready(&self, target: &str, timeout: Duration) -> Result<(i32, i32), String> {
		let deadline = Instant::now() + timeout;
		loop {
			if !self.animating()? {
				let mut error = String::new();
				for (find, what) in targets::finders(target) {
					match self.locate(&find, &what) {
						Ok(point) => return Ok(point),
						Err(e) => error = e,
					}
				}
				if Instant::now() >= deadline {
					return Err(error);
				}
			} else if Instant::now() >= deadline {
				return Err(format!(
					"the page was still animating when {target:?} was due"
				));
			}
			self.run_frames(0.1)?;
		}
	}

	/// Click `target` (see [`UiSession::locate_when_ready`]) once it's ready.
	pub fn click_when_ready(&self, target: &str, timeout: Duration) -> Result<(), String> {
		let (x, y) = self.locate_when_ready(target, timeout)?;
		self.click_at(x, y)
	}

	/// Double-click `target` (see [`UiSession::locate_when_ready`]) once it's ready.
	pub fn double_click_when_ready(&self, target: &str, timeout: Duration) -> Result<(), String> {
		let (x, y) = self.locate_when_ready(target, timeout)?;
		self.double_click_at(x, y)
	}

	/// Move the mouse over `target` (see [`UiSession::locate_when_ready`]) once it's
	/// ready, and leave it there.
	pub fn hover_when_ready(&self, target: &str, timeout: Duration) -> Result<(), String> {
		let (x, y) = self.locate_when_ready(target, timeout)?;
		self.move_to(x, y)
	}

	/// Turn the mouse wheel `notches` times over the first element matching the CSS
	/// `selector`; positive scrolls up.
	pub fn wheel_over(&self, selector: &str, notches: i32) -> Result<(), String> {
		let (x, y) = self.locate(
			&targets::selector(selector),
			&format!("element matching {selector:?}"),
		)?;
		self.input("wheel", |host| {
			host.wheel(x, y, notches);
			1
		})
	}

	/// Press and release `key` in the focused element.
	pub fn press_key(&self, key: Key) -> Result<(), String> {
		self.input("keyup", |host| {
			host.press_key(key);
			1
		})
	}

	/// Type `text` into the focused element (click it first), a key press per
	/// character. See [`Key`] for the codes the page sees.
	pub fn type_text(&self, text: &str) -> Result<(), String> {
		self.input("keyup", |host| {
			host.type_text(text);
			text.chars().count()
		})
	}

	/// Replace the text in the first text field matching the CSS `selector`, as a
	/// player would: click into it once it's ready, delete what's there with Backspace
	/// and Delete (wherever the caret landed), and type `text`.
	pub fn fill_field(&self, selector: &str, text: &str) -> Result<(), String> {
		self.click_when_ready(&format!("css:{selector}"), FIELD_TIMEOUT)?;
		let length = self
			.eval(&format!(
				"document.activeElement && document.activeElement.matches({}) ? document.activeElement.value.length : null",
				Json::Str(selector.to_string()).to_text()
			))?
			.as_i64()
			.ok_or_else(|| format!("clicking {selector:?} didn't focus it"))?;
		self.input("keyup", |host| {
			for key in [Key::Backspace, Key::Delete] {
				for _ in 0..length {
					host.press_key(key);
				}
			}
			2 * length as usize
		})?;
		self.type_text(text)
	}
}
