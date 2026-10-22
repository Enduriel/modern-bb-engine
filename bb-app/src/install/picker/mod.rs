//! The window that asks the player for the game's folder when bb found none: a page
//! of our own (not the game's UI, which is in the game's files) in the browser host,
//! saying where bb looked, with a box for the folder, the system's folder dialog
//! (`Browse…`, [`folder_dialog`]) and `Play`. The folder is checked as it's given
//! (`bb_engine::install::chosen`); one without the game is refused on the page, with
//! why. Closing the window or `Quit` ends bb.
//!
//! The browser host starts once a process (CEF can't start again after it shut
//! down), so the game doesn't start in this process: the caller remembers the folder
//! and starts bb again (`super::relaunch`).
//!
//! For the tests: `--shot FRAMES OUT.png` saves the window after that many of its
//! frames (counted from the page's first paint) and quits, `--at-page FRAMES JS` runs
//! the JS in the page then, and with `BB_TEST_FOLDER_DIALOG=DIR` the folder dialog
//! answers `DIR` without opening.

mod folder_dialog;
mod page;
mod view;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use bb_engine::install::Looked;
use bb_ui::bridge::Reply;
use bb_ui::host::Event;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::WindowId;

use folder_dialog::{Answer, FolderDialog};
use page::Action;
use view::View;

/// The variable that answers the folder dialog in the tests.
pub const TEST_DIALOG_VAR: &str = "BB_TEST_FOLDER_DIALOG";

/// How often the window looks for what the page sent while nothing else happens: the
/// browser's events come on a channel that can't wake the loop. Only while this
/// window is up, which is waiting for the player anyway.
const POLL: Duration = Duration::from_millis(16);

/// How long the page may take to paint once it's open, before bb gives up on it.
const FIRST_PAINT: Duration = Duration::from_secs(10);

/// What the window runs with, from the command line (see the module's notes).
#[derive(Clone, Debug, Default)]
pub struct Options {
	pub shot: Option<(u64, PathBuf)>,
	pub at_page: Vec<(u64, String)>,
	/// Where CEF writes its log.
	pub cef_log: Option<PathBuf>,
}

/// Ask the player for the game's folder, saying bb looked in `looked`: the data folder
/// chosen (checked), or `None` if they closed the window.
pub fn ask(looked: &[Looked], options: Options) -> Result<Option<PathBuf>, String> {
	let event_loop = EventLoop::new().map_err(|e| e.to_string())?;
	let display = Box::new(event_loop.owned_display_handle());
	let instance = wgpu::Instance::new(bb_render::Gpu::platform_backends(
		wgpu::InstanceDescriptor::new_with_display_handle_from_env(display),
	));
	let mut picker = Picker {
		instance,
		url: page::url(looked),
		options,
		view: None,
		opened: Instant::now(),
		frames: 0,
		dialog: FolderDialog::default(),
		ended: None,
	};
	event_loop.run_app(&mut picker).map_err(|e| e.to_string())?;
	// The page and the window go before the caller starts the game again.
	drop(picker.view.take());
	picker.ended.unwrap_or(Ok(None))
}

struct Picker {
	instance: wgpu::Instance,
	url: String,
	options: Options,
	view: Option<View>,
	/// When the page opened (it must paint within [`FIRST_PAINT`]).
	opened: Instant,
	/// The window's frames since the page first painted.
	frames: u64,
	/// The folder dialog (one at a time).
	dialog: FolderDialog,
	/// How it ended: the data folder chosen, `None` (closed), or an error.
	ended: Option<Result<Option<PathBuf>, String>>,
}

impl Picker {
	fn end(&mut self, event_loop: &ActiveEventLoop, ended: Result<Option<PathBuf>, String>) {
		self.ended.get_or_insert(ended);
		event_loop.exit();
	}

	/// Answer what the page sent since the last time, and run what that asks.
	fn serve_page(&mut self) -> Result<(), String> {
		let Some(view) = &self.view else { return Ok(()) };
		let mut actions = Vec::new();
		let mut scripts: Vec<String> = self.dialog.poll().iter().map(page::answer_script).collect();
		for event in view.host().poll() {
			match event {
				Event::Request { body, responder, .. } => view.host().serve(responder, &body, &mut collect(&mut actions)),
				Event::FolderChosen(folder) => scripts.push(page::answer_script(&self.dialog.browser_answered(folder))),
				// The page's own problems (its source is the whole page, as a `data:` URL).
				Event::Console { level, message, line, .. } if level != "info" => eprintln!("install page {level}: {message} (line {line})"),
				Event::LoadError { url, error } => return Err(format!("the install page didn't load ({error}): {}", &url[..url.len().min(60)])),
				Event::RendererGone(why) => return Err(format!("the install page crashed: {why}")),
				_ => {}
			}
		}
		self.run(actions, scripts)
	}

	/// Run `actions` and `scripts` in the page, then what those ask in turn (a folder
	/// the dialog picked is used, as if typed), until nothing is left or bb is done.
	fn run(&mut self, mut actions: Vec<Action>, mut scripts: Vec<String>) -> Result<(), String> {
		while !actions.is_empty() || !scripts.is_empty() {
			for action in std::mem::take(&mut actions) {
				match action {
					Action::Browse(typed) => self.browse(&typed, &mut scripts),
					Action::Use(folder) => match bb_engine::install::chosen(&expand_home(&folder)) {
						Ok(data_dir) => {
							self.ended = Some(Ok(Some(data_dir)));
							return Ok(());
						}
						Err(why) => scripts.push(page::refused_script(&why)),
					},
					Action::Quit => {
						self.ended = Some(Ok(None));
						return Ok(());
					}
				}
			}
			let Some(view) = &self.view else { return Ok(()) };
			for script in std::mem::take(&mut scripts) {
				view.host().eval(&script, &mut collect(&mut actions))?;
			}
		}
		Ok(())
	}

	/// Open the folder dialog, at the folder typed if there is one.
	fn browse(&mut self, typed: &str, scripts: &mut Vec<String>) {
		if let Some(answer) = std::env::var_os(TEST_DIALOG_VAR) {
			scripts.push(page::answer_script(&Answer::Folder(PathBuf::from(answer))));
			return;
		}
		let Some(view) = &self.view else { return };
		let typed = expand_home(typed);
		let start = (!typed.as_os_str().is_empty() && typed.is_dir()).then_some(typed.as_path());
		self.dialog.open(view.host(), "Choose the folder Battle Brothers is installed in", start);
	}

	/// A tick of the window: the page's requests, its frame if it painted, and the
	/// tests' scripts and shot.
	fn tick(&mut self, event_loop: &ActiveEventLoop) -> Result<(), String> {
		self.serve_page()?;
		if self.ended.is_some() {
			event_loop.exit();
			return Ok(());
		}
		let Some(view) = &mut self.view else { return Ok(()) };
		if let Some(failure) = view.paint_failure() {
			return Err(failure);
		}
		if !view.painted() {
			if self.opened.elapsed() > FIRST_PAINT {
				return Err(crate::requirements::page_error(format!(
					"{}: no paint within {FIRST_PAINT:?}",
					bb_ui::host::NO_PAINTS
				)));
			}
			return Ok(());
		}
		self.frames += 1;
		if view.changed() {
			view.window().request_redraw();
		}
		let due: Vec<String> = self.options.at_page.iter().filter(|(at, _)| *at == self.frames).map(|(_, code)| code.clone()).collect();
		self.run(Vec::new(), due)?;
		if self.ended.is_some() {
			event_loop.exit();
			return Ok(());
		}
		let Some(view) = &mut self.view else { return Ok(()) };
		if let Some((after, path)) = &self.options.shot {
			if self.frames >= *after {
				view.draw();
				view.save(path)?;
				eprintln!("wrote {}", path.display());
				self.end(event_loop, Ok(None));
			}
		}
		Ok(())
	}
}

/// A page call handler that keeps the page's actions in `actions` and answers nothing.
fn collect(actions: &mut Vec<Action>) -> impl FnMut(&bb_ui::json::Json) -> Reply + '_ {
	move |message| {
		if let Some(action) = page::action(message) {
			actions.push(action);
		}
		Reply::Void
	}
}

/// `~` at the start of a typed folder as the user's home.
fn expand_home(typed: &str) -> PathBuf {
	let home = || std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from);
	match typed.strip_prefix('~') {
		Some(rest) if rest.is_empty() || rest.starts_with(['/', '\\']) => match home() {
			Some(home) => home.join(rest.trim_start_matches(['/', '\\'])),
			None => PathBuf::from(typed),
		},
		_ => PathBuf::from(typed),
	}
}

impl ApplicationHandler for Picker {
	fn resumed(&mut self, event_loop: &ActiveEventLoop) {
		if self.view.is_some() || self.ended.is_some() {
			return;
		}
		event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + POLL));
		match View::open(event_loop, &self.instance, self.url.clone(), self.options.cef_log.clone()) {
			Ok(view) => {
				self.opened = Instant::now();
				self.view = Some(view);
			}
			Err(e) => self.end(event_loop, Err(e)),
		}
	}

	fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
		let Some(view) = &mut self.view else { return };
		match event {
			WindowEvent::CloseRequested => self.end(event_loop, Ok(None)),
			WindowEvent::Resized(size) => view.resize(size),
			WindowEvent::RedrawRequested => view.draw(),
			WindowEvent::Occluded(false) => view.window().request_redraw(),
			WindowEvent::CursorMoved { position, .. } => view.mouse_moved(position.x, position.y),
			WindowEvent::MouseInput { state, button, .. } => view.mouse_button(button, state.is_pressed()),
			WindowEvent::MouseWheel { delta, .. } => view.wheel(match delta {
				winit::event::MouseScrollDelta::LineDelta(_, y) => y.round() as i32,
				winit::event::MouseScrollDelta::PixelDelta(p) => (p.y / 40.0).round() as i32,
			}),
			WindowEvent::ModifiersChanged(m) => view.set_modifiers(m.state()),
			WindowEvent::KeyboardInput { event, is_synthetic: false, .. } => view.key(&event),
			_ => {}
		}
	}

	fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
		if let Some(view) = &self.view {
			// The browser's work, on macOS (this thread is its UI thread).
			view.host().pump();
		}
		if let Err(e) = self.tick(event_loop) {
			self.end(event_loop, Err(e));
			return;
		}
		let next = Instant::now() + POLL;
		let due = self.view.as_ref().and_then(|v| v.host().pump_due());
		event_loop.set_control_flow(ControlFlow::WaitUntil(due.map_or(next, |due| due.min(next))));
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_typed_tilde_is_the_home_folder() {
		let Some(home) = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }) else { return };
		let home = PathBuf::from(home);
		assert_eq!(expand_home("~/games/BB"), home.join("games/BB"));
		assert_eq!(expand_home("~"), home);
		assert_eq!(expand_home("/games/~x"), PathBuf::from("/games/~x"));
		assert_eq!(expand_home("~other/x"), PathBuf::from("~other/x"));
	}
}
