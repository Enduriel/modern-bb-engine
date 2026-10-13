//! [`UiSession`]: the real game with its real UI, headless: an install's scripts on
//! the [`Engine`](squirrel_bb::Engine), its `ui/` in a [`UiHost`], joined by the
//! [`UiProtocol`]. What the examples and integration tests drive.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};

use bb_engine::harness::{self, GameEngine};

use super::{Event, HostConfig, Key, RequestKind, UiHost};
use crate::bridge::UiProtocol;
use crate::coui::UiFiles;
use crate::json::Json;

/// How long the page may take to see key presses.
const KEY_TIMEOUT: Duration = Duration::from_secs(5);

/// How long [`UiSession::fill_field`] waits for its field to show up.
const FIELD_TIMEOUT: Duration = Duration::from_secs(30);

/// One line of what the page or the bridge reported.
#[derive(Clone, Debug)]
pub struct LogLine {
	/// `"error"`, `"warning"`, `"info"` (page console) or `"bridge"`.
	pub level: String,
	pub text: String,
}

/// The game and its UI page, running together on this thread.
pub struct UiSession {
	// Field order is drop order: the scripts (which hold the bridge's frontend) go
	// before the page.
	engine: GameEngine,
	protocol: Rc<UiProtocol<UiHost>>,
	log: Rc<RefCell<Vec<LogLine>>>,
	echo: bool,
}

impl UiSession {
	/// Start the page for the install at `data_dir` (its `data/` directory) and the
	/// script engine, and wait until the page has registered its screens. With `echo`,
	/// page and bridge messages are also printed as they arrive.
	pub fn start(data_dir: &Path, config: HostConfig, echo: bool) -> Result<UiSession, String> {
		let files = UiFiles::open_install(data_dir, false).map_err(|e| format!("archives: {e}"))?;
		let host = UiHost::start(files, config)?;
		let mut engine = harness::open_install(data_dir).map_err(|e| format!("install: {e}"))?;
		if echo {
			// Script errors print their full call stack.
			engine.vm_mut().set_print_handler(|s| eprint!("{s}"));
			engine.vm_mut().install_standard_error_handlers();
		}
		let log: Rc<RefCell<Vec<LogLine>>> = Rc::default();
		let sink = log.clone();
		let protocol = UiProtocol::new(host, move |text| {
			if echo {
				println!("  [bridge] {text}");
			}
			sink.borrow_mut().push(LogLine {
				level: "bridge".into(),
				text: text.to_string(),
			});
		});
		protocol.attach(engine.ui());
		let session = UiSession {
			engine,
			protocol,
			log,
			echo,
		};
		// The page registers its screens once loaded; scripts may only connect to them
		// after that, as in the original.
		if !session.pump_until(Duration::from_secs(30), |s| s.protocol.is_registered()) {
			return Err("the page never finished registering its screens".into());
		}
		Ok(session)
	}

	/// Boot the scripts to the main menu (config, then `scripts/root_state`).
	pub fn boot_menu(&self) -> Result<(), String> {
		harness::boot_menu(&self.engine)?;
		self.pump();
		Ok(())
	}

	pub fn engine(&self) -> &GameEngine {
		&self.engine
	}

	pub fn host(&self) -> &UiHost {
		self.protocol.transport()
	}

	pub fn protocol(&self) -> &UiProtocol<UiHost> {
		&self.protocol
	}

	/// Everything the page and the bridge reported so far.
	pub fn log(&self) -> Vec<LogLine> {
		self.log.borrow().clone()
	}

	fn record(&self, level: &str, text: String) {
		if self.echo {
			println!("  [{level}] {text}");
		}
		self.log.borrow_mut().push(LogLine {
			level: level.to_string(),
			text,
		});
	}

	/// React to one event from the page.
	fn handle(&self, event: Event) {
		match event {
			Event::Request {
				kind: RequestKind::Call,
				body,
				responder,
			} => {
				let (vm, bridge) = (self.engine.vm(), self.engine.ui());
				self.host().serve(responder, &body, &mut |message| {
					self.protocol.handle_message(vm, bridge, message)
				});
			}
			Event::Request {
				kind: RequestKind::Resume,
				..
			} => self.record(
				"bridge",
				"a result arrived for a call that had timed out".into(),
			),
			Event::Console {
				level,
				message,
				source,
				line,
			} => {
				let file = source.rsplit('/').next().unwrap_or_default();
				self.record(&level, format!("{message} ({file}:{line})"));
			}
			Event::LoadError { url, error } => {
				self.record("error", format!("failed to load {url}: {error}"))
			}
			Event::RendererGone(why) => self.record("error", format!("the page crashed: {why}")),
			_ => {}
		}
	}

	/// Handle everything the page sent so far (its calls into the scripts run now).
	pub fn pump(&self) {
		for event in self.host().poll() {
			self.handle(event);
		}
	}

	/// Handle the page's events as they come until `deadline`.
	fn pump_until_deadline(&self, deadline: Instant) {
		self.pump();
		while let Some(left) = deadline.checked_duration_since(Instant::now()) {
			match self.host().next_event(left) {
				Some(event) => self.handle(event),
				None => break,
			}
		}
	}

	/// Pump until `done` holds (true) or `timeout` passes (false).
	pub fn pump_until(&self, timeout: Duration, done: impl Fn(&Self) -> bool) -> bool {
		let deadline = Instant::now() + timeout;
		loop {
			self.pump_until_deadline((Instant::now() + Duration::from_millis(5)).min(deadline));
			if done(self) {
				return true;
			}
			if Instant::now() >= deadline {
				return false;
			}
		}
	}

	/// Run the game loop for `seconds` of real time, at about 60 frames per second,
	/// answering the page between frames as its calls arrive.
	pub fn run_frames(&self, seconds: f64) -> Result<(), String> {
		let frame = Duration::from_micros(16_667);
		let until = Instant::now() + Duration::from_secs_f64(seconds);
		let mut last = Instant::now();
		while Instant::now() < until {
			let now = Instant::now();
			self.engine
				.frame(now.duration_since(last).as_secs_f64() * 1000.0)?;
			last = now;
			self.pump_until_deadline((now + frame).min(until));
		}
		Ok(())
	}

	/// Evaluate `code` in the page; its calls into the scripts run inline.
	pub fn eval(&self, code: &str) -> Result<Json, String> {
		let (vm, bridge) = (self.engine.vm(), self.engine.ui());
		self.host().eval(code, &mut |message| {
			self.protocol.handle_message(vm, bridge, message)
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

	/// Run `find` (see [`UiSession::locate`]) and click there, as a player would: a
	/// real mouse click.
	fn click_found(&self, find: &str, what: &str) -> Result<(), String> {
		let (x, y) = self.locate(find, what)?;
		self.host().click(x, y);
		Ok(())
	}

	/// Send key presses with `send`, which returns how many, then pump until the page
	/// has seen all of them released (key events reach the page asynchronously).
	fn keys(&self, send: impl FnOnce(&UiHost) -> usize) -> Result<(), String> {
		let before = self
			.eval(
				"(function () {
	if (!window.__bbKeyups) {
		window.__bbKeyups = 0;
		window.addEventListener('keyup', function () { window.__bbKeyups++; }, true);
	}
	return window.__bbKeyups;
})()",
			)?
			.as_i64()
			.unwrap_or(0);
		let expected = before + send(self.host()) as i64;
		let seen = || {
			self.eval("window.__bbKeyups")
				.ok()
				.and_then(|n| n.as_i64())
				.is_some_and(|n| n >= expected)
		};
		if self.pump_until(KEY_TIMEOUT, |_| seen()) {
			Ok(())
		} else {
			Err("the page didn't get the key presses".into())
		}
	}

	/// Press and release `key` in the focused element.
	pub fn press_key(&self, key: Key) -> Result<(), String> {
		self.keys(|host| {
			host.press_key(key);
			1
		})
	}

	/// Type `text` into the focused element (click it first), a key press per
	/// character. See [`Key`] for the codes the page sees.
	pub fn type_text(&self, text: &str) -> Result<(), String> {
		self.keys(|host| {
			host.type_text(text);
			text.chars().count()
		})
	}

	/// Replace the text in the first visible text field matching the CSS `selector`, as
	/// a player would: click into it once it's ready, delete what's there with
	/// Backspace and Delete (wherever the caret landed), and type `text`.
	pub fn fill_field(&self, selector: &str, text: &str) -> Result<(), String> {
		self.click_when_ready(&format!("css:{selector}"), FIELD_TIMEOUT)?;
		let length = self
			.eval(&format!(
				"document.activeElement && document.activeElement.matches({0}) ? document.activeElement.value.length : null",
				Json::Str(selector.to_string()).to_text()
			))?
			.as_i64()
			.ok_or_else(|| format!("clicking {selector:?} didn't focus it"))?;
		self.keys(|host| {
			for key in [Key::Backspace, Key::Delete] {
				for _ in 0..length {
					host.press_key(key);
				}
			}
			2 * length as usize
		})?;
		self.type_text(text)
	}

	/// Turn the mouse wheel `notches` times over the first visible element matching
	/// the CSS `selector`; positive scrolls up. Runs the game for a moment after, so
	/// the page handles it.
	pub fn wheel_over(&self, selector: &str, notches: i32) -> Result<(), String> {
		let (x, y) = self.locate(
			&selector_centre(selector),
			&format!("element matching {selector:?}"),
		)?;
		self.host().wheel(x, y, notches);
		self.run_frames(0.1)
	}

	/// Click the visible `.ui-control` (button) whose text is `label`, at its centre.
	pub fn click_button(&self, label: &str) -> Result<(), String> {
		let find = format!(
			r#"(function (label) {{
	var button = Array.prototype.find.call(document.querySelectorAll('.ui-control'), function (el) {{
		return el.offsetParent !== null && el.textContent.trim() === label;
	}});
	if (!button) return null;
	var r = button.getBoundingClientRect();
	return [Math.round(r.left + r.width / 2), Math.round(r.top + r.height / 2)];
}})({})"#,
			Json::Str(label.to_string()).to_text()
		);
		self.click_found(&find, &format!("button labelled {label:?}"))
	}

	/// Click the innermost visible element whose text is `text` (a list entry, a
	/// label, a tab…), at its centre.
	pub fn click_text(&self, text: &str) -> Result<(), String> {
		let find = format!(
			r#"(function (text) {{
	var all = document.querySelectorAll('body *');
	for (var i = all.length - 1; i >= 0; --i) {{
		var el = all[i];
		if (el.offsetParent === null || el.textContent.trim() !== text) continue;
		var r = el.getBoundingClientRect();
		if (r.width === 0 || r.height === 0) continue;
		return [Math.round(r.left + r.width / 2), Math.round(r.top + r.height / 2)];
	}}
	return null;
}})({})"#,
			Json::Str(text.to_string()).to_text()
		);
		self.click_found(&find, &format!("element with the text {text:?}"))
	}

	/// Click the first visible element matching the CSS `selector` (e.g. an image
	/// button: `img[src$="icon_end_turn.png"]`), at its centre.
	pub fn click_selector(&self, selector: &str) -> Result<(), String> {
		self.click_found(
			&selector_centre(selector),
			&format!("element matching {selector:?}"),
		)
	}

	/// Click `target` once it is visible and the page has stopped animating, running
	/// the game meanwhile, as a player waits for a screen before clicking. `target` is
	/// `css:SELECTOR`, or else a button's label or any element's text. Fails after
	/// `timeout`.
	pub fn click_when_ready(&self, target: &str, timeout: Duration) -> Result<(), String> {
		let deadline = Instant::now() + timeout;
		loop {
			if !self.animating()? {
				let clicked = match target.strip_prefix("css:") {
					Some(selector) => self.click_selector(selector),
					None => self
						.click_button(target)
						.or_else(|_| self.click_text(target)),
				};
				match clicked {
					Ok(()) => return Ok(()),
					Err(error) if Instant::now() >= deadline => return Err(error),
					Err(_) => {}
				}
			} else if Instant::now() >= deadline {
				return Err(format!(
					"the page was still animating when {target:?} was due"
				));
			}
			self.run_frames(0.1)?;
		}
	}

	/// Whether some visible text on the page contains `text` (within one text node).
	pub fn shows_text(&self, text: &str) -> Result<bool, String> {
		let code = format!(
			r#"(function (text) {{
	var walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT, null, false);
	for (var node = walker.nextNode(); node; node = walker.nextNode()) {{
		var parent = node.parentElement;
		if (parent && parent.offsetParent !== null && node.nodeValue.indexOf(text) >= 0) return true;
	}}
	return false;
}})({})"#,
			Json::Str(text.to_string()).to_text()
		);
		Ok(self.eval(&code)? == Json::Bool(true))
	}

	/// Run the game until some visible element shows `text` and the page has stopped
	/// animating. Fails after `timeout`.
	pub fn wait_for_text(&self, text: &str, timeout: Duration) -> Result<(), String> {
		let deadline = Instant::now() + timeout;
		loop {
			if self.shows_text(text)? && !self.animating()? {
				return Ok(());
			}
			if Instant::now() >= deadline {
				return Err(format!("{text:?} never appeared"));
			}
			self.run_frames(0.1)?;
		}
	}

	/// The page as [`crate::parity`] compares it with the original's goldens.
	pub fn snapshot(&self) -> Result<crate::parity::Snapshot, String> {
		let code = format!(
			"(function () {{ {} return BBSnapshot.take(); }})()",
			crate::parity::SNAPSHOT_JS
		);
		let text = self.eval(&code)?;
		let json = Json::parse(text.as_str().ok_or("the snapshot is not text")?)?;
		crate::parity::Snapshot::parse(&json)
	}

	/// Whether the page is mid-animation (jQuery or Velocity, which the UI uses to
	/// slide and fade its screens): clicks then may land where a moving element was.
	pub fn animating(&self) -> Result<bool, String> {
		let busy = self.eval(
			r#"(function () {
				if (!window.jQuery) return false;
				var velocity = jQuery.Velocity;
				var calls = velocity && velocity.State ? velocity.State.calls : [];
				return jQuery(':animated').length > 0 || calls.some(function (call) { return !!call; });
			})()"#,
		)?;
		Ok(busy == Json::Bool(true))
	}

	/// The labels of every visible button, in document order.
	pub fn visible_buttons(&self) -> Result<Vec<String>, String> {
		let labels = self.eval(
			r#"Array.prototype.filter.call(document.querySelectorAll('.ui-control'), function (el) {
				return el.offsetParent !== null && el.querySelector('.ui-control') === null;
			}).map(function (el) { return el.textContent.trim(); }).filter(function (t) { return t !== ''; })"#,
		)?;
		Ok(labels
			.as_array()
			.unwrap_or_default()
			.iter()
			.filter_map(|l| l.as_str().map(str::to_string))
			.collect())
	}

	/// Save the latest painted frame as a PNG; its size, or `None` if nothing was
	/// painted yet.
	pub fn screenshot(&self, path: &Path) -> Result<Option<(u32, u32)>, String> {
		let Some(frame) = self.host().frame() else {
			return Ok(None);
		};
		let mut file =
			std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
		crate::png::write_bgra(&mut file, frame.width, frame.height, &frame.bgra)
			.map_err(|e| e.to_string())?;
		Ok(Some((frame.width, frame.height)))
	}
}

/// JS evaluating to the centre `[x, y]` of the first visible element matching the CSS
/// `selector`, or `null`.
fn selector_centre(selector: &str) -> String {
	format!(
		r#"(function (selector) {{
	var found = Array.prototype.find.call(document.querySelectorAll(selector), function (el) {{
		var r = el.getBoundingClientRect();
		return el.offsetParent !== null && r.width > 0 && r.height > 0;
	}});
	if (!found) return null;
	var r = found.getBoundingClientRect();
	return [Math.round(r.left + r.width / 2), Math.round(r.top + r.height / 2)];
}})({})"#,
		Json::Str(selector.to_string()).to_text()
	)
}
