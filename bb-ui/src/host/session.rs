//! [`UiSession`]: the real game with its real UI, headless: an install's scripts on
//! the [`Engine`](squirrel_bb::Engine), its `ui/` in a [`UiHost`], joined by the
//! [`UiProtocol`]. What the examples and integration tests drive.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bb_engine::harness::{self, GameEngine};

use super::page_log::{Echo, LogLine, PageLog};
use super::{Event, HostConfig, RequestKind, UiHost};
use crate::bridge::UiProtocol;
use crate::coui::UiFiles;
use crate::hot_reload::FileWatcher;
use crate::json::Json;

/// The game and its UI page, running together on this thread.
pub struct UiSession {
	// Field order is drop order: the scripts (which hold the bridge's frontend) go
	// before the page.
	engine: GameEngine,
	protocol: Rc<UiProtocol<UiHost>>,
	/// What the page is served from (shared with the host's `coui://` handler).
	files: Arc<UiFiles>,
	log: Rc<RefCell<PageLog>>,
	echo: Echo,
	/// Procedural images the page asked for, kept for the owner to answer
	/// ([`UiSession::queue_procedural`]); `None` answers them blank at once.
	pub(super) procedural: RefCell<Option<Vec<super::ProceduralRequest>>>,
	/// The fonts mods registered that the page has (`ModernEngine.registerFont`).
	fonts: RefCell<crate::mod_fonts::PageFonts>,
}

impl UiSession {
	/// Start the page for the install at `data_dir` (its `data/` directory) and the
	/// script engine, and wait until the page has registered its screens. Page and
	/// bridge messages are kept in [`log`](Self::log) and echoed as `echo` says.
	pub fn start(data_dir: &Path, config: HostConfig, echo: Echo) -> Result<UiSession, String> {
		let mut archives = harness::data_archives(data_dir).map_err(|e| format!("install: {e}"))?;
		if config.mods {
			archives.extend(harness::mod_archives(data_dir).map_err(|e| format!("mods: {e}"))?);
		}
		archives.extend(config.extra_mods.iter().cloned());
		let files = Arc::new(UiFiles::open(&archives, data_dir.parent().map(Path::to_path_buf)).map_err(|e| format!("archives: {e}"))?);
		let (screen_width, screen_height) = (config.width, config.height);
		let (exact_clock, battle_camera, battle_motion) = (config.exact_clock, config.battle_camera, config.battle_motion);
		let steam_build = config.steam_build;
		let host = UiHost::start(files.clone(), config)?;
		let configure = |c: &mut squirrel_bb::NativeConfig| {
			c.exact_clock = exact_clock;
			c.battle_camera = battle_camera;
			c.battle_motion = battle_motion;
			c.is_steam_build = steam_build;
		};
		// The scripts see the same archives as the page.
		let mut engine = harness::open_mounted(data_dir, archives, configure).map_err(|e| format!("install: {e}"))?;
		// The scripts see the window's size as the video mode's (the page fills it).
		engine
			.video()
			.set_screen_size(i64::from(screen_width), i64::from(screen_height));
		if echo == Echo::All {
			engine.vm_mut().set_print_handler(|s| eprint!("{s}"));
		}
		if echo != Echo::Off {
			// Script errors print their full call stack (to stderr, and log.html when
			// one is open), as the original logs them, including those in the page's
			// calls into the scripts.
			squirrel_bb::report_script_errors(engine.vm_mut());
		}
		let log: Rc<RefCell<PageLog>> = Rc::default();
		let sink = log.clone();
		let protocol = UiProtocol::new(host, move |text| {
			squirrel_bb::html_log::line(squirrel_bb::html_log::Class::Error, "UI", text);
			let line = LogLine {
				level: "bridge".into(),
				text: text.to_string(),
			};
			echo.line(&line);
			sink.borrow_mut().push(line);
		});
		protocol.attach(engine.ui());
		let session = UiSession {
			engine,
			protocol,
			files,
			log,
			echo,
			procedural: RefCell::new(None),
			fonts: RefCell::default(),
		};
		// The page registers its screens once loaded; scripts may only connect to them
		// after that, as in the original.
		if !session.pump_until(Duration::from_secs(30), |s| s.protocol.is_registered()) {
			return Err("the page never finished registering its screens".into());
		}
		// The page is painted on the GPU only: without paints nothing of it would ever
		// show, and Chromium fails that silently ("Unable to allocate frame" in its log).
		if !session.pump_until(Duration::from_secs(10), |s| s.host().paints() > 0) {
			let why = session.host().paint_error().map_or_else(|| "none arrived".to_string(), |e| format!("the last couldn't be taken: {e}"));
			return Err(format!(
				"the page never painted ({why}): Chromium's GPU process didn't hand over its paints; \
				its log is CEF's (cef.log, or the game's log folder)"
			));
		}
		Ok(session)
	}

	/// Boot the scripts to the main menu (config, then `scripts/root_state`). With
	/// Modern Hooks, the page first loads every mod's JS and then resumes the boot (its
	/// handshake), so this runs the page for up to 10 s until it has. Then the page runs
	/// until the scripts it inserted have all run (up to 30 s): mods' late JS (Modern
	/// Hooks' `registerLateJS`) loads after the handshake, and connects screens the
	/// scripts use from then on (MSU's settings page, which a new campaign shows). A
	/// player can't click anything before that; a host that starts a campaign at once
	/// mustn't either.
	pub fn boot_menu(&self) -> Result<(), String> {
		self.park();
		harness::boot_menu_with(&self.engine, 600, || {
			self.pump_until_deadline(Instant::now() + Duration::from_micros(16_667));
			Ok(())
		})?;
		self.pump();
		if !self.pump_until(Duration::from_secs(30), |s| s.scripts_settled()) {
			self.record("bridge", "the page's inserted scripts were still loading after 30 s".into());
		}
		Ok(())
	}

	/// Whether every script the page inserted has run (or failed to load).
	fn scripts_settled(&self) -> bool {
		matches!(self.eval("window.__bbScriptsPending === 0"), Ok(Json::Bool(true)))
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

	/// What the page and the bridge reported so far: the latest lines, a bounded
	/// number of them, problems kept longest.
	pub fn log(&self) -> Vec<LogLine> {
		self.log.borrow().lines()
	}

	pub(super) fn record(&self, level: &str, text: String) {
		let line = LogLine {
			level: level.to_string(),
			text,
		};
		self.echo.line(&line);
		self.log.borrow_mut().push(line);
	}

	/// React to one event from the page.
	pub(super) fn handle(&self, event: Event) {
		match event {
			Event::Request {
				kind: RequestKind::Call | RequestKind::Task,
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
				if level == "error" {
					page_error_to_log(&message, &source, line);
				}
				let file = source.rsplit('/').next().unwrap_or_default();
				self.record(&level, format!("{message} ({file}:{line})"));
			}
			Event::LoadError { url, error } => {
				let text = format!("failed to load {url}: {error}");
				squirrel_bb::html_log::line(squirrel_bb::html_log::Class::Error, "UI", &text);
				self.record("error", text)
			}
			Event::RendererGone(why) => {
				let text = format!("the page crashed: {why}");
				squirrel_bb::html_log::line(squirrel_bb::html_log::Class::Error, "UI", &text);
				self.record("error", text)
			}
			Event::Procedural { path, responder } => match self.procedural.borrow_mut().as_mut() {
				Some(queue) => queue.push(super::ProceduralRequest { path, responder }),
				None => responder.respond_image(None),
			},
			_ => {}
		}
	}

	/// Park the page (see [`UiHost::park`]): its own tasks (animation frames, timers,
	/// input) wait until the session pumps, as the original's UI waits for the game's
	/// thread. The session keeps the page parked whenever the scripts may run: every
	/// method returns with it parked, but [`unpark`](Self::unpark).
	pub fn park(&self) {
		let (vm, bridge) = (self.engine.vm(), self.engine.ui());
		let parked = self
			.host()
			.park(&mut |message| self.protocol.handle_message(vm, bridge, message));
		if let Err(error) = parked {
			self.record("bridge", format!("the page didn't park: {error}"));
		}
		self.report_notes();
	}

	/// Let the page run its own tasks until the session parks it again (before the
	/// scripts next run). For a game loop that draws while the page works.
	pub fn unpark(&self) {
		self.host().unpark();
	}

	fn report_notes(&self) {
		for note in self.host().take_notes() {
			self.record("bridge", note);
		}
	}

	/// Let the page run until it has done what it had to (its calls into the scripts
	/// run now), then park it again.
	pub fn pump(&self) {
		self.unpark();
		for event in self.host().poll() {
			self.handle(event);
		}
		self.park();
	}

	/// Let the page run, handling its events as they come until `deadline`, then park
	/// it again.
	fn pump_until_deadline(&self, deadline: Instant) {
		self.unpark();
		// No game frame is drawn while the session waits: the page's frames go on.
		self.host().pace();
		for event in self.host().poll() {
			self.handle(event);
		}
		while let Some(left) = deadline.checked_duration_since(Instant::now()) {
			match self.host().next_event(left) {
				Some(event) => self.handle(event),
				None => break,
			}
		}
		self.park();
	}

	/// Pump until `done` holds (true) or `timeout` passes (false). `done` runs with the
	/// page parked, so it may run scripts.
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

	/// Run `seconds` of the game loop: that many 60th-of-a-second frames, each given
	/// exactly 1/60 s, answering the page between them. Frames are paced in real time
	/// (the page's animations run on it), but a slow frame makes the run longer instead
	/// of the game different, so what the scripts do doesn't depend on the machine.
	pub fn run_frames(&self, seconds: f64) -> Result<(), String> {
		let frame = Duration::from_micros(16_667);
		let frames = (seconds * 60.0).round().max(1.0) as u64;
		self.park();
		for _ in 0..frames {
			let start = Instant::now();
			self.game_frame(1000.0 / 60.0)?;
			self.follow_ui_scale();
			self.follow_fonts();
			self.pump_until_deadline(start + frame);
		}
		Ok(())
	}

	/// Lay the page out at the video mode's size divided by its UI scale, as the original
	/// does when the scripts set a video mode (`Settings.setVideoMode`, e.g. the Options
	/// screen's UI scale): the page sees a smaller window, and is drawn
	/// at the screen's resolution.
	pub(super) fn follow_ui_scale(&self) {
		let (width, height) = self.engine.video().ui_size();
		let (_, _, scale) = self.engine.video().screen();
		let scale = if scale > 0.0 { scale } else { 1.0 };
		let size = (width as i32, height as i32);
		if self.host().size() != size || self.host().scale() != scale {
			self.host().resize(size.0, size.1, scale);
		}
	}

	/// Give the page the fonts mods registered since it last looked (`registerFont`), as
	/// `@font-face` rules for vanilla's families ([`crate::mod_fonts`]).
	pub(super) fn follow_fonts(&self) {
		let new = self.engine.font_registrations(self.fonts.borrow().len());
		if new.is_empty() {
			return;
		}
		let script = {
			let mut fonts = self.fonts.borrow_mut();
			for font in &new {
				fonts.add(font);
			}
			fonts.page_script()
		};
		if let Err(e) = self.eval(&script) {
			self.record("bridge", format!("mods' fonts not on the page: {e}"));
		}
	}

	/// Evaluate `code` in the page; its calls into the scripts run inline.
	pub fn eval(&self, code: &str) -> Result<Json, String> {
		let (vm, bridge) = (self.engine.vm(), self.engine.ui());
		self.host().eval(code, &mut |message| {
			self.protocol.handle_message(vm, bridge, message)
		})
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

	/// Restyle the running page: every `<link>`ed style sheet is fetched again (a new
	/// query defeats the cache; `coui://` ignores it), and the page's state stays as it
	/// is. How many sheets were reloaded.
	pub fn reload_styles(&self) -> Result<usize, String> {
		let reloaded = self.eval(
			r#"(function () {
	var stamp = Date.now();
	var links = document.querySelectorAll('link[rel=stylesheet]');
	Array.prototype.forEach.call(links, function (link) {
		var bare = (link.getAttribute('href') || '').split('?')[0];
		link.setAttribute('href', bare + '?bb-reload=' + stamp);
	});
	return links.length;
})()"#,
		)?;
		Ok(reloaded.as_i64().unwrap_or(0) as usize)
	}

	/// A watcher over the archives the page is served from, for [`UiSession::hot_reload`].
	pub fn watch_ui(&self) -> FileWatcher {
		FileWatcher::new(self.files.archive_paths().iter().cloned())
	}

	/// Hot reload: if `watcher` saw an archive change (a mod author rebuilt
	/// their `mod_*.zip`), open the archives again and restyle the page with them. The
	/// archives that changed.
	pub fn hot_reload(&self, watcher: &mut FileWatcher) -> Result<Vec<PathBuf>, String> {
		let changed = watcher.changed();
		if !changed.is_empty() {
			self.files
				.reopen()
				.map_err(|e| format!("reopening the archives: {e}"))?;
			self.reload_styles()?;
		}
		Ok(changed)
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

/// A page error in log.html, as the original logs its UI's (probed): only
/// errors (its `console.log`, `info` and `warn` go nowhere), tagged `UI`, as
/// `ConsoleAPI <url>:<line>:<column>: <message>` for `console.error` and `JS …` for an
/// uncaught exception. Chromium reports no column, so ours is `<url>:<line>: `.
fn page_error_to_log(message: &str, source: &str, line: i32) {
	let text = match message.strip_prefix("Uncaught ") {
		Some(rest) => format!("JS {source}:{line}: {rest}"),
		None => format!("ConsoleAPI {source}:{line}: {message}"),
	};
	squirrel_bb::html_log::line(squirrel_bb::html_log::Class::Error, "UI", text);
}
