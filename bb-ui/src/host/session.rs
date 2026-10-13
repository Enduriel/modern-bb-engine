//! [`UiSession`]: the real game with its real UI, headless: an install's scripts on
//! the [`Engine`](squirrel_bb::Engine), its `ui/` in a [`UiHost`], joined by the
//! [`UiProtocol`]. What the examples and integration tests drive.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};

use bb_engine::harness::{self, GameEngine};

use super::{Event, HostConfig, RequestKind, UiHost};
use crate::bridge::UiProtocol;
use crate::coui::UiFiles;
use crate::json::Json;

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
		let engine = harness::open_install(data_dir).map_err(|e| format!("install: {e}"))?;
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

	/// Click the visible `.ui-control` (button) whose text is `label`, as a player
	/// would: a real mouse click at its centre.
	pub fn click_button(&self, label: &str) -> Result<(), String> {
		let code = format!(
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
		let target = self.eval(&code)?;
		let xy: Option<Vec<i64>> = target
			.as_array()
			.map(|xy| xy.iter().filter_map(Json::as_i64).collect());
		let Some([x, y]) = xy.and_then(|xy| <[i64; 2]>::try_from(xy).ok()) else {
			return Err(format!("no visible button labelled {label:?}"));
		};
		self.host().click(x as i32, y as i32);
		Ok(())
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
