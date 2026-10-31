//! The update notice, end to end, against a local server standing in for GitHub (never
//! the internet): bb boots to the main menu, its check asks the server through the
//! browser's network stack, and the newer release's notice shows on the menu, with both
//! versions and the release notes as text. It is modal: it covers the page, and keys
//! don't reach the states (Escape is "Continue" and Enter "Open link", neither reaching
//! the states, their releases included). Each of its buttons, clicked as a player's mouse
//! does: "Continue" closes it; "Open link" hands the release's page to the URL opener and
//! closes it; "Ignore this version" keeps the version in the (test's own) settings file,
//! after which that version no longer shows but a later one does. An answer that comes
//! while the main menu is hidden waits for it to show again.
//!
//! Skips without a Steam install or a GPU.
//!
//! Run: `cargo test -p bb-app --features ui --test update_notice`
//!
//! A plain `main` (`harness = false`): the browser host re-runs this binary for its
//! child processes, and starts only once per process.

use std::cell::RefCell;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bb_app::game::{Boot, Game, Start};
use bb_app::update_check::UpdateCheck;
use bb_engine::harness::STEAM_DATA_DIR;
use bb_render::Gpu;
use bb_ui::json::Json;
use squirrel::SqTable;
use squirrel_bb::VideoMode;

const NAME: &str = "the_update_notice_shows_on_the_main_menu";
/// The engine's key codes for Escape and Enter, and a key the main menu does nothing
/// with (`A`).
const ESCAPE: i64 = bb_app::keys::ESCAPE;
const ENTER: i64 = bb_app::keys::ENTER;
const OTHER_KEY: i64 = 11;
const RELEASES: &str = "https://github.com/Enduriel/modern-bb-engine/releases/tag/";

fn ensure(condition: bool, what: impl std::fmt::Display) -> Result<(), String> {
	if condition { Ok(()) } else { Err(what.to_string()) }
}

/// GitHub's answer for release `tag`.
fn release(tag: &str) -> String {
	Json::Object(vec![
		("tag_name".into(), Json::Str(tag.into())),
		("html_url".into(), Json::Str(format!("{RELEASES}{tag}"))),
		("draft".into(), Json::Bool(false)),
		("prerelease".into(), Json::Bool(false)),
		("body".into(), Json::Str("## What's new\r\n- **Faster** loading\r\n- <b>not bold</b><img src=x onerror=alert(1)>".into())),
	])
	.to_text()
}

/// A local stand-in for GitHub's API: every request gets `answer`'s release, and is
/// counted.
struct Server {
	url: String,
	answer: Arc<Mutex<String>>,
	served: Arc<AtomicUsize>,
}

impl Server {
	fn start(tag: &str) -> Server {
		let listener = TcpListener::bind("127.0.0.1:0").expect("a local port");
		let url = format!("http://127.0.0.1:{}/repos/Enduriel/modern-bb-engine/releases/latest", listener.local_addr().unwrap().port());
		let answer = Arc::new(Mutex::new(release(tag)));
		let served = Arc::new(AtomicUsize::new(0));
		let (body, count) = (answer.clone(), served.clone());
		std::thread::spawn(move || {
			for stream in listener.incoming() {
				let Ok(mut stream) = stream else { continue };
				let mut head = Vec::new();
				let mut byte = [0u8; 1];
				while !head.ends_with(b"\r\n\r\n") && stream.read(&mut byte).is_ok_and(|n| n == 1) {
					head.push(byte[0]);
				}
				let body = body.lock().unwrap().clone();
				let response = format!(
					"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
					body.len()
				);
				let _ = stream.write_all(response.as_bytes());
				count.fetch_add(1, Ordering::SeqCst);
			}
		});
		Server { url, answer, served }
	}

	fn answer(&self, tag: &str) {
		*self.answer.lock().unwrap() = release(tag);
	}
}

fn frame(game: &mut Game) -> Result<(), String> {
	game.step(1000.0 / 60.0)?;
	std::thread::sleep(Duration::from_millis(16));
	Ok(())
}

/// Run frames until `done` holds, failing as `what` after `timeout`.
fn frames_until(game: &mut Game, timeout: Duration, what: &str, done: impl Fn(&Game) -> bool) -> Result<(), String> {
	let deadline = Instant::now() + timeout;
	while !done(game) {
		if Instant::now() > deadline {
			return Err(format!("{what} (within {timeout:?})"));
		}
		frame(game)?;
	}
	Ok(())
}

/// Run frames until the server has answered `count` requests in all, and a second more
/// for the answer to be decided.
fn answered(game: &mut Game, server: &Server, count: usize) -> Result<(), String> {
	frames_until(game, Duration::from_secs(15), "the server got no request", |_| server.served.load(Ordering::SeqCst) >= count)?;
	for _ in 0..60 {
		frame(game)?;
	}
	Ok(())
}

/// What the notice on the page says (its text, by part), or `null` if none shows.
fn notice(game: &Game) -> Result<Json, String> {
	game.front().session().eval(
		"(function () {
			var host = document.querySelector('bb-engine-update-notice');
			if (!host) { return null; }
			var root = host.shadowRoot;
			var text = function (s) { var e = root.querySelector(s); return e ? e.textContent : null; };
			return { found: text('.found'), current: text('.current'), notes: text('.notes'),
				markup: root.querySelectorAll('.notes b, .notes img, .notes script').length,
				buttons: Array.prototype.map.call(root.querySelectorAll('button'), function (b) { return b.textContent; }) };
		})()",
	)
}

/// The middle of the notice's button for `choice`, in the game's screen pixels.
fn button_at(game: &Game, choice: &str) -> Result<(i64, i64), String> {
	let at = game.front().session().eval(&format!(
		"(function () {{
			var host = document.querySelector('bb-engine-update-notice');
			var b = host && host.shadowRoot.querySelector('button[data-choice=\"{choice}\"]');
			if (!b) {{ return null; }}
			var r = b.getBoundingClientRect();
			return [Math.round(r.left + r.width / 2), Math.round(r.top + r.height / 2)];
		}})()"
	))?;
	match at.as_array() {
		Some([x, y]) => Ok((x.as_i64().unwrap_or(0), y.as_i64().unwrap_or(0))),
		_ => Err(format!("no {choice} button on the page")),
	}
}

/// Click the notice's `choice` as a player does: the pointer there, the left button down
/// and up, a few frames.
fn click(game: &mut Game, choice: &str) -> Result<(), String> {
	let (x, y) = button_at(game, choice)?;
	game.mouse_moved(x, y)?;
	frame(game)?;
	game.mouse_button(1, true)?;
	game.mouse_button(1, false)?;
	frames_until(game, Duration::from_secs(5), &format!("{choice} left the notice up"), |g| !g.front().update_notice_shown())?;
	for _ in 0..10 {
		frame(game)?;
	}
	ensure(notice(game)? == Json::Null, format!("{choice}: the notice is still on the page"))
}

/// The notice comes up on the menu for version `found`, says what it should, and is
/// modal.
fn shows(game: &mut Game, found: &str) -> Result<(), String> {
	frames_until(game, Duration::from_secs(20), &format!("the notice for {found} never showed"), |g| g.front().update_notice_shown())?;
	let shown = notice(game)?;
	let field = |name: &str| shown.get(name).and_then(Json::as_str).unwrap_or("").to_string();
	ensure(field("found") == found, format!("the version found: {shown:?}"))?;
	ensure(field("current") == env!("CARGO_PKG_VERSION"), format!("the version running: {shown:?}"))?;
	let notes = field("notes");
	ensure(notes.contains("What's new") && notes.contains("Faster loading"), format!("the notes: {notes:?}"))?;
	ensure(notes.contains("<b>not bold</b><img src=x onerror=alert(1)>"), format!("the notes' HTML isn't shown as text: {notes:?}"))?;
	ensure(shown.get("markup") == Some(&Json::Int(0)), format!("the notes made elements: {shown:?}"))?;
	let buttons = shown.get("buttons").map(Json::to_text).unwrap_or_default();
	ensure(buttons == r#"["Open link","Continue","Ignore this version"]"#, format!("the buttons: {buttons}"))?;
	// Modal: it covers the page (the mouse goes to it everywhere, the corner too).
	frames_until(game, Duration::from_secs(5), "the notice doesn't cover the page's corner", |g| g.front().session().is_over_ui(2, 2))?;
	Ok(())
}

/// The key with the engine's `code` pressed and released, and a few frames.
fn press(game: &mut Game, code: i64) -> Result<(), String> {
	for down in [true, false] {
		game.key(&bb_app::front::Key { engine: Some(code), text: None, down, repeat: false })?;
		for _ in 0..5 {
			frame(game)?;
		}
	}
	Ok(())
}

/// A Squirrel condition, evaluated in the scripts.
fn holds(game: &Game, expr: &str) -> Result<bool, String> {
	game.run_script(&format!("::__holds <- ({expr}) ? true : false;"))?;
	SqTable::root(game.front().engine().vm()).get::<bool>("__holds").ok_or_else(|| "no answer".to_string())
}

fn settings_text(path: &Path) -> String {
	std::fs::read_to_string(path).unwrap_or_default()
}

fn check(gpu: &Gpu, dir: &Path) -> Result<(), String> {
	let settings = dir.join("settings.cfg");
	std::fs::write(&settings, "# the player's own line\nui.show_fps = false\n").map_err(|e| e.to_string())?;
	let server = Server::start("v9.0.0");
	let boot = Boot {
		data_dir: Path::new(STEAM_DATA_DIR),
		start: Start::Menu,
		mods: false,
		sound: false,
		world_size: None,
		mode: VideoMode::DEFAULT,
		settings: None,
		devtools: None,
		saves: None,
		cef_log: None,
		store: Default::default(),
		update_check: Some(UpdateCheck { endpoint: server.url.clone(), settings: Some(settings.clone()) }),
		display: None,
	};
	let mut game = Game::start(gpu, &boot)?;
	let opened = Rc::new(RefCell::new(Vec::new()));
	let sink = opened.clone();
	// The test's opener in place of the system browser's.
	game.front().session().protocol().set_url_opener(move |url| {
		sink.borrow_mut().push(url.to_string());
		Ok(())
	});

	// At the start: the notice on the menu. Keys don't reach the states while it shows
	// (Escape on the main menu goes back, or quits): the main menu's onKeyInput counts
	// what it gets. Any key but Escape and Enter does nothing.
	shows(&mut game, "9.0.0")?;
	game.keyboard_focus(true);
	game.run_script(
		"::__keys <- 0; local menu = ::RootState.get(\"MainMenuState\"); local on = menu.onKeyInput; \
		 menu.onKeyInput <- function (_key) { ::__keys++; return on(_key); };",
	)?;
	press(&mut game, OTHER_KEY)?;
	ensure(game.front().update_notice_shown(), "another key closed the notice")?;
	ensure(holds(&game, "::__keys == 0")?, "a key reached the states under the notice")?;
	// Escape is Continue: it closes the notice, keeping nothing, and neither its press
	// nor its release (after the notice has gone) reaches the menu.
	press(&mut game, ESCAPE)?;
	ensure(!game.front().update_notice_shown() && notice(&game)? == Json::Null, "Escape left the notice up")?;
	ensure(!game.quit() && holds(&game, "::__keys == 0")?, "Escape reached the states")?;
	ensure(!settings_text(&settings).contains("update."), format!("Escape kept something: {}", settings_text(&settings)))?;
	ensure(opened.borrow().is_empty(), "Escape opened a link")?;
	// Once it's gone, keys reach the states again (a key the menu does nothing with).
	press(&mut game, OTHER_KEY)?;
	ensure(holds(&game, "::__keys > 0")?, "keys don't reach the states after the notice")?;

	// Asked again (as at the next start): its Continue closes it, keeping nothing.
	game.check_for_update();
	shows(&mut game, "9.0.0")?;
	click(&mut game, "continue")?;
	ensure(!settings_text(&settings).contains("update."), format!("Continue kept something: {}", settings_text(&settings)))?;
	ensure(opened.borrow().is_empty(), "Continue opened a link")?;

	// Enter is Open link, and reaches the states no more than Escape.
	game.check_for_update();
	shows(&mut game, "9.0.0")?;
	game.run_script("::__keys = 0;")?;
	press(&mut game, ENTER)?;
	ensure(!game.front().update_notice_shown() && notice(&game)? == Json::Null, "Enter left the notice up")?;
	ensure(holds(&game, "::__keys == 0")?, "Enter reached the states")?;
	ensure(*opened.borrow() == [format!("{RELEASES}v9.0.0")], format!("Enter: the opener got {:?}", opened.borrow()))?;

	// The Open link button does the same.
	game.check_for_update();
	shows(&mut game, "9.0.0")?;
	click(&mut game, "open")?;
	ensure(*opened.borrow() == [format!("{RELEASES}v9.0.0"), format!("{RELEASES}v9.0.0")], format!("the opener got {:?}", opened.borrow()))?;

	// Ignore this version: kept in the settings file, beside the player's lines.
	game.check_for_update();
	shows(&mut game, "9.0.0")?;
	click(&mut game, "ignore")?;
	let text = settings_text(&settings);
	ensure(
		text.starts_with("# the player's own line\nui.show_fps = false\n") && text.contains("\nupdate.ignored_version = 9.0.0\n"),
		format!("the settings file after Ignore: {text:?}"),
	)?;
	// That version no longer shows; a later one does.
	let served = server.served.load(Ordering::SeqCst);
	game.check_for_update();
	answered(&mut game, &server, served + 1)?;
	ensure(!game.front().update_notice_shown() && notice(&game)? == Json::Null, "the ignored version showed")?;
	server.answer("v9.1.0");
	game.check_for_update();
	shows(&mut game, "9.1.0")?;
	click(&mut game, "continue")?;

	// An answer while the main menu is hidden waits for it.
	game.run_script("::RootState.get(\"MainMenuState\").hide();")?;
	let served = server.served.load(Ordering::SeqCst);
	game.check_for_update();
	answered(&mut game, &server, served + 1)?;
	ensure(!game.front().update_notice_shown() && notice(&game)? == Json::Null, "the notice showed without the main menu")?;
	game.run_script("::RootState.get(\"MainMenuState\").show();")?;
	ensure(holds(&game, "::RootState.get(\"MainMenuState\").isVisible()")?, "the main menu didn't show again")?;
	shows(&mut game, "9.1.0")?;
	click(&mut game, "continue")?;
	println!("  {} requests served", server.served.load(Ordering::SeqCst));
	Ok(())
}

fn main() -> ExitCode {
	if let Some(code) = bb_ui::host::run_subprocess() {
		return ExitCode::from(code.clamp(0, 255) as u8);
	}
	// A runner's `--list` (cargo-nextest's), answered before a display starts.
	if let Some(code) = bb_engine::harness::list_case(NAME) {
		return code;
	}
	// Chromium paints the page through a display connection: a headless gamescope's.
	bb_ui::host::with_display(run_main)
}

fn run_main() -> ExitCode {
	let data = Path::new(STEAM_DATA_DIR);
	if !data.exists() {
		return bb_engine::harness::skip_case(NAME, format!("{STEAM_DATA_DIR} not present"));
	}
	let gpu = match Gpu::headless_with(bb_app::front::GPU_FEATURES) {
		Ok(gpu) => gpu,
		Err(e) => return bb_engine::harness::skip_case(NAME, format!("no GPU: {e}")),
	};
	// The test's own settings folder, never the player's.
	let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("update_notice-{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&dir);
	if let Err(e) = std::fs::create_dir_all(&dir) {
		println!("test {NAME} ... FAILED\n{}: {e}", dir.display());
		return ExitCode::FAILURE;
	}
	let result = check(&gpu, &dir);
	let _ = std::fs::remove_dir_all(&dir);
	match result {
		Ok(()) => {
			println!("test {NAME} ... ok");
			ExitCode::SUCCESS
		}
		Err(e) => {
			println!("test {NAME} ... FAILED\n{e}");
			ExitCode::FAILURE
		}
	}
}
