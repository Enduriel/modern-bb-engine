//! End-to-end through the real UI: the game's `ui/` runs in the Chromium host
//! (offscreen, headless) against the real scripts, boots to the main menu, and a real
//! mouse click on "New Campaign" opens the new-campaign menu, whose origin list the
//! mouse wheel scrolls and which "Cancel" closes; Options round-trips its settings;
//! calls between scripts and page run inline both ways, as in the original, and the
//! page's own tasks run between the scripts, never inside one. Images with nothing to
//! show paint nothing and right clicks open no menu, as in Coherent. Skips if there is
//! no Steam install.
//!
//! Run: `cargo test -p bb-ui --features cef --test menu_ui`
//!
//! A plain `main` (`harness = false`): Chromium re-runs this binary for its child
//! processes, which must go straight to [`run_subprocess`](bb_ui::host::run_subprocess),
//! and CEF can only start once per process.

use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use bb_engine::harness::STEAM_DATA_DIR;
use bb_ui::host::{HostConfig, MouseButton, UiSession};
use bb_ui::json::Json;
use squirrel::SqTable;

fn ensure(condition: bool, what: impl std::fmt::Display) -> Result<(), String> {
	if condition {
		Ok(())
	} else {
		Err(what.to_string())
	}
}

/// Run the game until a button labelled `label` is visible and the page has stopped
/// animating, as a player would wait before clicking. Fails after `timeout`.
/// Set the UI scale as the Options screen does, and let the page lay itself out again.
fn set_ui_scale(session: &UiSession, scale: f64) -> Result<(), String> {
	session.engine().run_script(
		&format!("local m = ::Settings.getVideoMode(); m.UIScale = {scale:.2}; ::Settings.setVideoMode(m);"),
		"ui_scale",
	)?;
	session.run_frames(1.0)
}

fn wait_for_button(session: &UiSession, label: &str, timeout: Duration) -> Result<(), String> {
	let deadline = Instant::now() + timeout;
	loop {
		let buttons = session.visible_buttons()?;
		if buttons.iter().any(|b| b == label) && !session.animating()? {
			return Ok(());
		}
		if Instant::now() > deadline {
			return Err(format!(
				"no {label:?} button appeared; visible: {buttons:?}"
			));
		}
		session.run_frames(0.1)?;
	}
}

fn boots_to_the_main_menu_and_opens_new_campaign(data_dir: &Path) -> Result<(), String> {
	let session = UiSession::start(data_dir, HostConfig::default(), false)?;
	let screens = session.protocol().screens();
	ensure(
		screens.len() == 22,
		format!("the page registered {} screens: {screens:?}", screens.len()),
	)?;

	session.boot_menu()?;
	wait_for_button(&session, "New Campaign", Duration::from_secs(10))?;
	let buttons = session.visible_buttons()?;
	for label in [
		"New Campaign",
		"Load Campaign",
		"Scenarios",
		"Options",
		"Credits",
		"Quit",
	] {
		ensure(
			buttons.iter().any(|b| b == label),
			format!("main menu lacks {label:?}: {buttons:?}"),
		)?;
	}

	// What the scripts sent (`setVersion`, `setMOTD`) is on the page.
	let page = session.eval(
		r#"({ version: document.querySelector('.main-menu-screen .version').textContent,
		      motd: document.querySelector('.main-menu-screen .motd').textContent })"#,
	)?;
	let version = page.get("version").and_then(|v| v.as_str()).unwrap_or("");
	ensure(
		version.starts_with("1."),
		format!("version text {version:?}"),
	)?;
	let motd = page.get("motd").and_then(|v| v.as_str()).unwrap_or("");
	ensure(
		motd.starts_with("Battle Brothers is a challenging game"),
		format!("motd {motd:?}"),
	)?;

	// A real click goes page → scripts → page: the menu state opens the new-campaign
	// module, with the origins the scenario manager listed.
	session.click_button("New Campaign")?;
	wait_for_button(&session, "Next", Duration::from_secs(10))?;
	let origins = session.eval(
		r#"document.querySelector('.main-menu-screen').textContent.indexOf('A New Company') >= 0"#,
	)?;
	ensure(
		origins.to_text() == "true",
		"the origin list is missing 'A New Company'",
	)?;

	// The wheel scrolls the origin list as far as the original's does. A notch is 30
	// ticks of 120 there (`WHEEL_DELTA_PER_NOTCH`), aciScrollBar reads jquery.mousewheel's
	// `wheelDelta / 120` and moves twice the list's delta (5) a tick: 300 CSS pixels a
	// notch, or to the end, at any UI scale.
	let list = ".new-campaign-menu-module .l-list-container .aciSb_cnt";
	let number = |code: String| {
		session.eval(&code).map(|v| match v {
			Json::Int(n) => n as f64,
			Json::Float(f) => f,
			_ => -1.0,
		})
	};
	let notch = f64::from(bb_ui::host::WHEEL_DELTA_PER_NOTCH / 120 * 2 * 5);
	ensure(notch == 300.0, format!("a notch is {notch} pixels"))?;
	// The page's own wheel events, before the list's handler takes them.
	session.eval(
		"window.__wheelDeltas = []; document.addEventListener('mousewheel', function (e) { window.__wheelDeltas.push(e.wheelDelta); }, true); 0",
	)?;
	for scale in [1.0, 1.5] {
		set_ui_scale(&session, scale)?;
		let end = number(format!(
			"(function (e) {{ return e.scrollHeight - e.clientHeight; }})(document.querySelector({list:?}))"
		))?;
		let top = || number(format!("document.querySelector({list:?}).scrollTop"));
		ensure(end > 0.0, format!("the origin list doesn't scroll at UI scale {scale}"))?;
		session.eval("window.__wheelDeltas = []; 0")?;
		session.wheel_over(list, -1)?;
		let deltas = session.eval("window.__wheelDeltas.join(',')")?;
		ensure(
			deltas.as_str() == Some("-3600"),
			format!("a notch down reached the page as wheelDelta {deltas:?} at UI scale {scale}, not -3600"),
		)?;
		let down = top()?;
		ensure(
			down == notch.min(end),
			format!("a notch down moved the origin list to {down} at UI scale {scale}, not {}", notch.min(end)),
		)?;
		session.wheel_over(list, 1)?;
		let up = top()?;
		ensure(up == 0.0, format!("a notch back up left the origin list at {up} at UI scale {scale}"))?;
	}
	set_ui_scale(&session, 1.0)?;

	session.click_button("Cancel")?;
	wait_for_button(&session, "New Campaign", Duration::from_secs(10))?;

	// Options: the scripts fill it from the display modes and settings, and "Ok"
	// applies every tab's settings back through them.
	session.click_button("Options")?;
	wait_for_button(&session, "1920 x 1080", Duration::from_secs(10))?;
	session.click_button("Ok")?;
	wait_for_button(&session, "New Campaign", Duration::from_secs(10))?;

	// Calls run inline both ways, as in the original: an `asyncCall` returns after the
	// page ran it, including the page's own call back into the scripts.
	session.eval(
		r#"Screens.MainMenuScreen.bbEcho = function (x) { SQ.call(this.mSQHandle, 'bbEchoBack', x + 1); }"#,
	)?;
	session.engine().run_script(
		r#"
		local screen = ::RootState.get("MainMenuState").m.MainMenuScreen;
		screen.bbEchoBack <- function (_x) { ::__bbEchoed <- _x; };
		::__bbEchoed <- null;
		screen.m.JSHandle.asyncCall("bbEcho", 41);
		if (::__bbEchoed != 42) throw "the page's call back arrived after asyncCall returned";
		screen.m.JSHandle.call("setMOTD", "inline MOTD");
		"#,
		"inline_calls",
	)?;
	let motd = session.eval("document.querySelector('.main-menu-screen .motd').textContent")?;
	ensure(
		motd.as_str() == Some("inline MOTD"),
		format!("after JSHandle.call the MOTD is {}", motd.to_text()),
	)?;

	// Hot reload restyles the running page: every style sheet loads again, and the
	// menu is laid out as before (the compat rewrites apply to the new sheets too).
	let button_box = "(function () { var r = document.querySelector('.main-menu-module .ui-control').getBoundingClientRect(); return [r.left, r.top, r.width, r.height]; })()";
	let before = session.eval(button_box)?;
	let sheets = session.reload_styles()?;
	ensure(sheets > 10, format!("only {sheets} style sheets reloaded"))?;
	session.run_frames(1.0)?;
	let after = session.eval(button_box)?;
	ensure(
		after == before,
		format!(
			"the first menu button moved from {} to {} after restyling",
			before.to_text(),
			after.to_text()
		),
	)?;

	// Procedural images (portraits) come from the game thread: the placeholder unless
	// it answers.
	ensure(
		load_image_width(&session, "coui://procedural/test(1)")? == 1,
		"an unanswered procedural image isn't the 1×1 placeholder",
	)?;
	session.queue_procedural(true);
	let width = Json::Str("coui://procedural/test(2)".into()).to_text();
	session.eval(&format!(
		"(function () {{ var i = new Image(); window.__bbWidth = null; i.onload = function () {{ window.__bbWidth = i.naturalWidth; }}; i.src = {width}; return true; }})()"
	))?;
	let deadline = Instant::now() + Duration::from_secs(10);
	let request = loop {
		if let Some(request) = session.take_procedural().pop() {
			break request;
		}
		ensure(
			Instant::now() < deadline,
			"the procedural image never reached the game",
		)?;
		session.run_frames(0.1)?;
	};
	ensure(
		request.path == "test(2)",
		format!("procedural path {:?}", request.path),
	)?;
	let mut png = Vec::new();
	bb_ui::png::write_rgba(&mut png, 3, 2, &[255; 24]).map_err(|e| e.to_string())?;
	request.answer(Some(png));
	session.queue_procedural(false);
	ensure(
		image_width(&session)? == 3,
		"the game's procedural image didn't reach the page",
	)?;

	images_with_nothing_to_show_paint_nothing(&session)?;
	right_clicks_reach_the_page(&session)?;
	page_tasks_run_between_scripts(&session)?;

	let problems: Vec<String> = session
		.log()
		.into_iter()
		.filter(|line| line.level == "error" || line.level == "bridge")
		.map(|line| format!("[{}] {}", line.level, line.text))
		.collect();
	ensure(
		problems.is_empty(),
		format!("page or bridge errors:\n{}", problems.join("\n")),
	)
}

/// The page's own tasks (here a timer) run between the scripts, as the original's UI
/// runs on the game's thread: a timer that keeps the page busy while a script calls
/// into the page reaches the scripts after that script, not in the middle of its call.
fn page_tasks_run_between_scripts(session: &UiSession) -> Result<(), String> {
	session.engine().run_script(
		r#"
		::__inScript <- false;
		::__fromTask <- [];
		::RootState.get("MainMenuState").m.MainMenuScreen.__fromTask <- function () { ::__fromTask.push(::__inScript); };
		"#,
		"listen",
	)?;
	session.eval(
		"setTimeout(function () {
			var until = Date.now() + 300;
			while (Date.now() < until) {}
			SQ.call(Screens.MainMenuScreen.mSQHandle, '__fromTask');
		}, 0); 0",
	)?;
	// Time for the timer to start, if the page may run now.
	std::thread::sleep(Duration::from_millis(50));
	session.engine().run_script(
		"::__inScript = true; ::Tooltip.hide(); ::__inScript = false;",
		"a script calling the page",
	)?;
	let calls = || {
		session
			.engine()
			.run_script("::__calls <- ::__fromTask.len();", "count")?;
		Ok::<_, String>(
			SqTable::root(session.engine().vm())
				.get::<i64>("__calls")
				.unwrap_or(0),
		)
	};
	session.pump_until(Duration::from_secs(5), |_| calls().is_ok_and(|n| n > 0));
	ensure(
		calls()? == 1,
		format!("the page's timer reached the scripts {} times", calls()?),
	)?;
	session
		.engine()
		.run_script("::__inside <- ::__fromTask[0];", "check")?;
	ensure(
		SqTable::root(session.engine().vm()).get::<bool>("__inside") == Some(false),
		"the page's timer reached the scripts inside a script's call into the page",
	)
}

/// Test images over the page, each named by `data-name`: an empty stash slot as the
/// game makes it (its overlay's `src` is empty), an image with no `src` (like the
/// character screen's talent stars), one whose file is missing, and one whose file is
/// missing until it gets one that loads.
const NOTHING_TO_SHOW: &str = r#"(function () {
	var page = $('<div id="bb-nothing"/>').css({ position: 'fixed', left: 0, top: 0, width: '460px', height: '120px', background: '#204060', 'z-index': 2147483647 });
	$('body').append(page);
	function cell(x) {
		var c = $('<div/>').css({ position: 'absolute', left: x + 'px', top: '10px', width: '100px', height: '100px' });
		page.append(c);
		return c;
	}
	cell(10).createListItem(false).data('item').overlay.attr('data-name', 'an empty stash slot');
	function image(x, name, src) {
		var image = $('<img/>').attr('data-name', name).css({ width: '60px', height: '60px' });
		if (src !== null) {
			image.attr('src', src);
		}
		cell(x).append(image);
	}
	image(120, 'no src', null);
	image(230, 'a missing file', Path.GFX + 'ui/bb_no_such_image.png');
	image(340, 'a file once loaded', Path.GFX + 'ui/bb_no_such_image.png');
	return true;
})()"#;

/// Images with nothing to show paint nothing, as in Coherent, where Chromium would
/// paint a broken-image icon in a box. One that failed and then loads shows.
fn images_with_nothing_to_show_paint_nothing(session: &UiSession) -> Result<(), String> {
	session.eval(NOTHING_TO_SHOW)?;
	let recovering = r#"document.querySelector('#bb-nothing img[data-name="a file once loaded"]')"#;
	ensure(
		load_complete(session, recovering)? == 0,
		"the missing file loaded",
	)?;
	session.eval(&format!(
		"{recovering}.src = Path.GFX + Asset.ICON_REPAIR_ITEM"
	))?;
	ensure(
		load_complete(session, recovering)? > 0,
		"the image's second file didn't load",
	)?;

	let painting = images_painting(session, "#bb-nothing img[data-name]")?;
	session.eval("$('#bb-nothing').remove(); true")?;
	let expected = [
		("an empty stash slot", false),
		("no src", false),
		("a missing file", false),
		("a file once loaded", true),
	];
	ensure(
		painting.len() == expected.len()
			&& expected
				.iter()
				.all(|(name, paints)| painting.iter().any(|(n, p)| n == name && p == paints)),
		format!("which images paint: {painting:?}, expected {expected:?}"),
	)
}

/// The natural width of `image` (a JS expression) once it has loaded or failed.
fn load_complete(session: &UiSession, image: &str) -> Result<i64, String> {
	let deadline = Instant::now() + Duration::from_secs(10);
	loop {
		if session.eval(&format!("{image}.complete"))? == Json::Bool(true) {
			return Ok(session
				.eval(&format!("{image}.naturalWidth"))?
				.as_i64()
				.unwrap_or(0));
		}
		ensure(
			Instant::now() < deadline,
			"the image never finished loading",
		)?;
		session.run_frames(0.1)?;
	}
}

/// Whether each image `selector` matches paints anything: the page's pixels in its
/// box, compared with those where every match is hidden. By `data-name`.
fn images_painting(session: &UiSession, selector: &str) -> Result<Vec<(String, bool)>, String> {
	let selector = Json::Str(selector.into()).to_text();
	let found = session.eval(&format!(
		"Array.prototype.map.call(document.querySelectorAll({selector}), function (image) {{
			var r = image.getBoundingClientRect();
			return [image.getAttribute('data-name'), Math.floor(r.left), Math.floor(r.top), Math.ceil(r.right), Math.ceil(r.bottom)];
		}})"
	))?;
	let Json::Array(found) = found else {
		return Err(format!("no images: {}", found.to_text()));
	};
	let pixels = |session: &UiSession| {
		session.run_frames(0.3)?;
		session
			.host()
			.frame()
			.ok_or_else(|| "nothing painted".to_string())
	};
	let shown = pixels(session)?;
	let visibility = |value: &str| {
		session.eval(&format!(
			"Array.prototype.forEach.call(document.querySelectorAll({selector}), function (image) {{ image.style.visibility = '{value}'; }}); true"
		))
	};
	visibility("hidden")?;
	let hidden = pixels(session);
	visibility("")?;
	let hidden = hidden?;

	let mut painting = Vec::new();
	for image in &found {
		let Json::Array(fields) = image else {
			return Err(format!("not an image: {}", image.to_text()));
		};
		let name = fields
			.first()
			.and_then(Json::as_str)
			.unwrap_or("?")
			.to_string();
		let edge = |i: usize, limit: u32| {
			let value = fields.get(i).and_then(Json::as_i64).unwrap_or(0);
			value.clamp(0, i64::from(limit)) as u32
		};
		let (left, top) = (edge(1, shown.width), edge(2, shown.height));
		let (right, bottom) = (edge(3, shown.width), edge(4, shown.height));
		let differs = (top..bottom).any(|y| {
			let row =
				((y * shown.width + left) * 4) as usize..((y * shown.width + right) * 4) as usize;
			shown.bgra[row.clone()] != hidden.bgra[row]
		});
		painting.push((name, differs));
	}
	Ok(painting)
}

/// The right mouse button reaches the page, and Chromium shows no menu of its own
/// (Coherent has none; an offscreen Chromium logs errors trying).
fn right_clicks_reach_the_page(session: &UiSession) -> Result<(), String> {
	session.eval(
		"window.__bbRight = [];
		['mousedown', 'mouseup', 'contextmenu'].forEach(function (type) {
			document.addEventListener(type, function (e) { if (e.button === 2) window.__bbRight.push(e.type); }, true);
		});
		true",
	)?;
	session.host().mouse_button(5, 5, MouseButton::Right, true);
	session.host().mouse_button(5, 5, MouseButton::Right, false);
	session.run_frames(0.3)?;
	let seen = session.eval("window.__bbRight.sort().join(' ')")?;
	ensure(
		seen.as_str() == Some("contextmenu mousedown mouseup"),
		format!("the page saw the right click as {}", seen.to_text()),
	)
}

/// The natural width of the image the page last loaded (see [`load_image_width`]),
/// once it has loaded.
fn image_width(session: &UiSession) -> Result<i64, String> {
	let deadline = Instant::now() + Duration::from_secs(10);
	loop {
		if let Json::Int(width) = session.eval("window.__bbWidth")? {
			return Ok(width);
		}
		ensure(Instant::now() < deadline, "the image never loaded")?;
		session.run_frames(0.1)?;
	}
}

/// Load the image at `url` in the page; its natural width.
fn load_image_width(session: &UiSession, url: &str) -> Result<i64, String> {
	let url = Json::Str(url.into()).to_text();
	session.eval(&format!(
		"(function () {{ var i = new Image(); window.__bbWidth = null; i.onload = function () {{ window.__bbWidth = i.naturalWidth; }}; i.src = {url}; return true; }})()"
	))?;
	image_width(session)
}

fn main() -> ExitCode {
	if let Some(code) = bb_ui::host::run_subprocess() {
		return ExitCode::from(code.clamp(0, 255) as u8);
	}
	let name = "boots_to_the_main_menu_and_opens_new_campaign";
	let data_dir = Path::new(STEAM_DATA_DIR);
	if !data_dir.exists() {
		println!(
			"test {name} ... skipped ({} not present)",
			data_dir.display()
		);
		return ExitCode::SUCCESS;
	}
	match boots_to_the_main_menu_and_opens_new_campaign(data_dir) {
		Ok(()) => {
			println!("test {name} ... ok");
			ExitCode::SUCCESS
		}
		Err(error) => {
			println!("test {name} ... FAILED\n{error}");
			ExitCode::FAILURE
		}
	}
}
