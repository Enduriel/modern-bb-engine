//! The update notice on the main menu (see [`crate::update_check`]): the check's
//! request through the browser's network stack, its answer decided, and the notice
//! shown on the page while the main menu is up.
//!
//! - The answer comes back on a channel, looked at once a frame until it has come
//!   (nothing after). Offline, a timeout, an HTTP error (rate limits among them), an
//!   answer that isn't a release: one line in the log, and nothing else happens.
//! - The main menu being up is `MainMenuState` shown, which the state stack reports as
//!   it happens ([`squirrel_bb::RootState::set_visibility_listener`]): an answer that
//!   comes during a campaign waits for the menu, and a notice whose menu goes away
//!   (only a script could do that under it) waits for it to come back.
//! - The notice's buttons answer through the engine's own handler
//!   ([`UPDATE_NOTICE_HANDLER`]), inside the page's call: "Ignore this version" keeps the
//!   version in the settings file there and then; each closes the notice. Its "Open
//!   link" goes through the page's `openURL` itself, as the game's links do.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use bb_ui::bridge::Reply;
use bb_ui::host::{Fetched, UPDATE_NOTICE_HANDLER, UiSession, UpdateNoticeView};
use bb_ui::json::Json;
use squirrel_bb::html_log::{self, Class};

use crate::settings::Settings;
use crate::update_check::{self, Notice, UpdateCheck, link::NEXUS_PAGE};

/// The state whose being shown is the main menu being up.
const MAIN_MENU: &str = "MainMenuState";

/// How long the request may take, all of it.
const TIMEOUT: Duration = Duration::from_secs(15);

/// The most of an answer taken (GitHub's for a release is a few KB, its notes included).
const MAX_ANSWER: usize = 1 << 20;

/// The log's tag for the check's lines.
const TAG: &str = "Update";

enum Phase {
	/// The request is out.
	Asking(mpsc::Receiver<Result<Fetched, String>>),
	/// A notice to show once the main menu is up.
	Waiting(Notice),
	/// The notice is on the page, until one of its buttons closes it.
	Showing(Notice),
	/// Nothing (more) to show.
	Done,
}

pub struct UpdateNotice {
	check: UpdateCheck,
	phase: Phase,
	/// Whether `MainMenuState` shows (the state stack's listener sets it).
	menu_up: Rc<Cell<bool>>,
	/// Whether the notice is on the page (its buttons' handler clears it).
	on_page: Rc<Cell<bool>>,
}

impl UpdateNotice {
	/// Follow the main menu and answer the notice's buttons in `session`. Before the
	/// scripts boot, so the menu's first showing is seen.
	pub fn attach(session: &UiSession, check: UpdateCheck) -> UpdateNotice {
		let menu_up = Rc::new(Cell::new(false));
		let up = menu_up.clone();
		session.engine().root_state().set_visibility_listener(move |name, shown| {
			if name == MAIN_MENU {
				up.set(shown);
			}
		});
		let on_page = Rc::new(Cell::new(false));
		let open = on_page.clone();
		let settings = check.settings.clone();
		session.protocol().set_engine_handler(UPDATE_NOTICE_HANDLER, move |args| {
			open.set(false);
			chosen(args, settings.as_ref());
			Reply::Void
		});
		UpdateNotice { check, phase: Phase::Done, menu_up, on_page }
	}

	/// Ask for the latest release (again): its notice shows on the main menu if it makes
	/// one. One showing is taken down.
	pub fn check(&mut self, session: &UiSession) {
		if matches!(self.phase, Phase::Showing(_)) {
			let _ = session.hide_update_notice();
			self.on_page.set(false);
		}
		let user_agent = format!("battle-brothers-modern/{}", env!("CARGO_PKG_VERSION"));
		let headers = [("User-Agent", user_agent.as_str()), ("Accept", "application/vnd.github+json")];
		self.phase = Phase::Asking(bb_ui::host::fetch(&self.check.endpoint, &headers, TIMEOUT, MAX_ANSWER));
	}

	/// Whether the notice is on the page: it takes the player's input while it is.
	pub fn shown(&self) -> bool {
		matches!(self.phase, Phase::Showing(_)) && self.on_page.get()
	}

	/// Each frame, before the scripts' step: take the answer if it has come, and show the
	/// notice (or take it down) as the main menu comes and goes.
	pub fn step(&mut self, session: &UiSession) {
		self.phase = match std::mem::replace(&mut self.phase, Phase::Done) {
			Phase::Asking(answer) => match answer.try_recv() {
				Ok(result) => self.answered(result),
				Err(mpsc::TryRecvError::Empty) => Phase::Asking(answer),
				Err(mpsc::TryRecvError::Disconnected) => {
					log("the update check got no answer");
					Phase::Done
				}
			},
			Phase::Waiting(notice) if self.menu_up.get() => self.show(session, notice),
			Phase::Showing(_) if !self.on_page.get() => Phase::Done,
			Phase::Showing(notice) if !self.menu_up.get() => {
				if !session.park_for_frame() {
					Phase::Showing(notice)
				} else {
					let _ = session.hide_update_notice();
					self.on_page.set(false);
					Phase::Waiting(notice)
				}
			}
			phase => phase,
		};
	}

	/// What the request's `result` comes to.
	fn answered(&self, result: Result<Fetched, String>) -> Phase {
		let decided = result.and_then(|fetched| self.decide(fetched));
		match decided {
			Ok(Some(notice)) => {
				log(&format!("version {} is out (this is {}): the main menu shows it", notice.version, notice.current));
				Phase::Waiting(notice)
			}
			Ok(None) => Phase::Done,
			Err(why) => {
				log(&format!("the update check failed: {why}"));
				Phase::Done
			}
		}
	}

	fn decide(&self, fetched: Fetched) -> Result<Option<Notice>, String> {
		if fetched.status != 200 {
			return Err(format!("HTTP status {}", fetched.status));
		}
		let text = String::from_utf8(fetched.body).map_err(|_| "the answer is not UTF-8".to_string())?;
		let release = update_check::release::parse(&text)?;
		let settings = Settings::load(self.check.settings.clone());
		update_check::decide(&release, env!("CARGO_PKG_VERSION"), settings.ignored_update(), NEXUS_PAGE)
	}

	/// Put `notice` on the page, once the page is parked as for the scripts' calls.
	fn show(&self, session: &UiSession, notice: Notice) -> Phase {
		if !session.park_for_frame() {
			return Phase::Waiting(notice);
		}
		let view = UpdateNoticeView {
			version: notice.version.clone(),
			current: notice.current.clone(),
			notes: notice.notes.clone(),
			link: notice.link.clone(),
		};
		match session.show_update_notice(&view) {
			Ok(()) => {
				self.on_page.set(true);
				Phase::Showing(notice)
			}
			Err(e) => {
				log(&format!("the update notice could not show: {e}"));
				Phase::Done
			}
		}
	}
}

/// One of the notice's buttons: `args` is `(choice[, version])`.
fn chosen(args: &[Json], settings: Option<&PathBuf>) {
	let choice = args.first().and_then(Json::as_str).unwrap_or("");
	if choice != "ignore" {
		return;
	}
	let Some(version) = args.get(1).and_then(Json::as_str).and_then(update_check::version::Version::parse) else {
		log("the update notice's Ignore came without a version");
		return;
	};
	let mut kept = Settings::load(settings.cloned());
	match kept.keep_ignored_update(&version.to_string()) {
		Ok(()) if settings.is_some() => log(&format!("version {version} is ignored from now on")),
		Ok(()) => log(&format!("version {version} is ignored, but there is no settings file to keep that in")),
		Err(e) => log(&format!("version {version} could not be kept as ignored: {e}")),
	}
}

fn log(text: &str) {
	html_log::engine(Class::Info, TAG, text);
}
