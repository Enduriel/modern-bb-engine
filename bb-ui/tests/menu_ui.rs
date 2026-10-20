//! End-to-end through the real UI: the game's `ui/` runs in the Chromium host
//! (offscreen, headless) against the real scripts, boots to the main menu, and a real
//! mouse click on "New Campaign" opens the new-campaign menu, whose origin list the
//! mouse wheel scrolls and which "Cancel" closes; Options round-trips its settings;
//! calls between scripts and page run inline both ways, as in the original, and the
//! page's own tasks run between the scripts, never inside one. Images with nothing to
//! show paint nothing, bold or italic text in a face without either draws in it as it
//! is, text advances by whole pixels, and right clicks open no menu, as in Coherent;
//! scripts inserted at runtime run in Coherent's order. The page's links reach the URL
//! opener the game installs; copy and paste go through the game's clipboard.
//! Where the game discards a scene, the page gives its memory back. Vanilla's FPS
//! module shows only when the game asks for it. Scroll bars follow their content without
//! polling it. Skips if there is no Steam install.
//!
//! Run: `cargo test -p bb-ui --features cef --test menu_ui`
//!
//! A plain `main` (`harness = false`): Chromium re-runs this binary for its child
//! processes, which must go straight to [`run_subprocess`](bb_ui::host::run_subprocess),
//! and CEF can only start once per process.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use bb_engine::harness::STEAM_DATA_DIR;
use bb_ui::clipboard::{Clipboard, MemoryClipboard};
use bb_ui::host::{Echo, HostConfig, Modifiers, MouseButton, UiSession};
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
	let clipboard = Arc::new(MemoryClipboard::default());
	let config = HostConfig { clipboard: clipboard.clone(), ..HostConfig::default() };
	let session = UiSession::start(data_dir, config, Echo::Off)?;
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
	for scale in [1.0, 1.25, 1.5] {
		set_ui_scale(&session, scale)?;
		drawn_at_the_screens_resolution(&session, scale, list)?;
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
	faces_are_never_faked(&session)?;
	text_lays_out_in_whole_pixels(&session)?;
	discards_give_memory_back(&session)?;
	right_clicks_reach_the_page(&session)?;
	page_tasks_run_between_scripts(&session)?;
	inserted_scripts_run_in_coherents_order(&session)?;
	fps_module_shows_when_asked(&session)?;
	scroll_bars_follow_their_content(&session)?;
	copy_and_paste_use_the_games_clipboard(&session, &clipboard)?;

	let problems: Vec<String> = session
		.log()
		.into_iter()
		.filter(|line| line.level == "error" || line.level == "bridge")
		.map(|line| format!("[{}] {}", line.level, line.text))
		.collect();
	ensure(
		problems.is_empty(),
		format!("page or bridge errors:\n{}", problems.join("\n")),
	)?;
	links_reach_the_url_opener(&session)
}

/// The page's list of a scroll bar (vanilla's `createList`), its bar and thumb.
const SCROLL_LIST: &str = r#"(function () {
	var list = window.__bbList, bar = list.find('.aciSb_bar_v:last'), content = list.find('.aciSb_cnt:first')[0];
	var track = list.find('.aciSb_track_v:last')[0], grip = list.find('.aciSb_drag_vc:last')[0];
	var reach = content.scrollHeight - content.clientHeight;
	var expected = reach > 0 ? content.scrollTop * (track.offsetHeight - grip.offsetHeight) / reach : 0;
	return { bar: bar.is(':visible'), grip: $(grip).is(':visible'), top: parseFloat(grip.style.top) || 0,
		expected: expected, scrollTop: content.scrollTop, updates: window.__bbUpdates };
})()"#;

/// Vanilla's scroll bars follow their content when it changes, without the plugin's
/// 200ms poll (an intentional divergence, docs/differences.md): a list's bar shows
/// within a few frames of rows being added, its thumb follows a scroll made around the
/// plugin and content growing, it hides when the rows go, and while nothing changes
/// nothing updates it.
fn scroll_bars_follow_their_content(session: &UiSession) -> Result<(), String> {
	session.eval(
		r#"(function () {
			var host = $('<div id="bb-scroll" style="position:absolute;left:0;top:0;width:200px;height:100px;z-index:99999"/>').appendTo('body');
			window.__bbList = host.createList(8);
			window.__bbUpdates = 0;
			window.__bbList.on('update', function () { window.__bbUpdates++; });
			return true;
		})()"#,
	)?;
	// The list three frames (50ms) after a change: a 200ms poll would mostly miss it.
	let state = || -> Result<Json, String> {
		session.run_frames(0.05)?;
		session.eval(SCROLL_LIST)
	};
	let number = |state: &Json, key: &str| match state.get(key) {
		Some(Json::Int(n)) => *n as f64,
		Some(Json::Float(f)) => *f,
		_ => f64::NAN,
	};
	session.run_frames(0.5)?;
	let empty = state()?;
	ensure(empty.get("bar") == Some(&Json::Bool(false)), format!("an empty list shows its bar: {}", empty.to_text()))?;

	session.eval("window.__bbUpdates = 0; 0")?;
	session.run_frames(1.0)?;
	let idle = session.eval("window.__bbUpdates")?;
	ensure(idle.as_i64() == Some(0), format!("an unchanged list updated its bar {} times in a second", idle.to_text()))?;

	// Rows added: the bar shows within three frames, far sooner than a 200ms poll.
	session.eval(
		r#"(function () {
			var rows = window.__bbList.findListScrollContainer();
			for (var i = 0; i < 20; i++) { rows.append($('<div class="bb-row" style="height:20px">row ' + i + '</div>')); }
			return true;
		})()"#,
	)?;
	let filled = state()?;
	ensure(
		filled.get("bar") == Some(&Json::Bool(true)) && filled.get("grip") == Some(&Json::Bool(true)),
		format!("rows added, but the bar or its thumb isn't shown: {}", filled.to_text()),
	)?;

	// Scrolled around the plugin (a script setting `scrollTop`): the thumb follows.
	session.eval("window.__bbList.find('.aciSb_cnt:first')[0].scrollTop = 150; 0")?;
	let scrolled = state()?;
	ensure(
		number(&scrolled, "scrollTop") == 150.0 && number(&scrolled, "top") > 0.0 &&
			(number(&scrolled, "top") - number(&scrolled, "expected")).abs() < 0.5,
		format!("the thumb didn't follow a scroll: {}", scrolled.to_text()),
	)?;

	// The content grows (a style change, no rows added): the thumb moves up to match.
	session.eval("window.__bbList.find('.bb-row').css('height', '40px'); 0")?;
	let grown = state()?;
	ensure(
		number(&grown, "top") < number(&scrolled, "top") &&
			(number(&grown, "top") - number(&grown, "expected")).abs() < 0.5,
		format!("the thumb didn't follow the content growing: {} after {}", grown.to_text(), scrolled.to_text()),
	)?;

	// The rows go: the bar hides again.
	session.eval("window.__bbList.findListScrollContainer().empty(); 0")?;
	let emptied = state()?;
	ensure(
		emptied.get("bar") == Some(&Json::Bool(false)),
		format!("the bar stayed after its rows went: {}", emptied.to_text()),
	)?;

	session.eval("window.__bbList.destroyList(); $('#bb-scroll').remove(); delete window.__bbList; delete window.__bbUpdates; true")?;
	Ok(())
}

/// The page's links go to the URL opener the game installs (the system's browser in
/// `bb`; here a stand-in that only records them): the version module's website link
/// (vanilla's `openURL`, with an address that has no scheme; the link is hidden on the
/// main menu, so its handler runs from jQuery), and a mod's
/// own `engine.call('openURL', ...)`, as MSU's Nexus Mods icon makes it. What isn't a
/// web address never reaches it.
fn links_reach_the_url_opener(session: &UiSession) -> Result<(), String> {
	let opened = Rc::new(RefCell::new(Vec::new()));
	let sink = opened.clone();
	session.protocol().set_url_opener(move |url| {
		sink.borrow_mut().push(url.to_string());
		Ok(())
	});
	session.eval(
		"$('.version-container .game-name').click(); \
		 engine.call('openURL', 'https://www.nexusmods.com/battlebrothers/mods/765'); \
		 engine.call('openURL', 'file:///etc/passwd'); 0",
	)?;
	session.run_frames(0.1)?;
	let opened = opened.borrow();
	ensure(
		opened.as_slice()
			== [
				"http://www.battlebrothersgame.com",
				"https://www.nexusmods.com/battlebrothers/mods/765",
			],
		format!("the opener got {opened:?}"),
	)?;
	let refused = session
		.log()
		.iter()
		.any(|line| line.text == r#"ui: openURL "file:///etc/passwd" (refused: not an http(s) URL)"#);
	ensure(refused, "the file URL wasn't logged as refused")
}

/// Copy and paste go through the game's clipboard (the system's in `bb`; here the
/// test's own), as Coherent's went through Windows': Plan Perks' Export and Paste
/// buttons (`select()` then `execCommand('copy')` / `('paste')`), Ctrl+C and Ctrl+V in
/// a field, and a page that puts its own text on the copy. Password fields never
/// copy out.
fn copy_and_paste_use_the_games_clipboard(session: &UiSession, clipboard: &MemoryClipboard) -> Result<(), String> {
	let eval = |code: &str| session.eval(code).map_err(|e| format!("{e}\nin: {code}"));
	let value = |id: &str| -> Result<String, String> {
		Ok(eval(&format!("document.getElementById('{id}').value"))?.as_str().unwrap_or("").to_string())
	};
	eval(
		"['bbClipA', 'bbClipB'].forEach(function (id) { var i = document.createElement('input'); i.type = 'text'; i.id = id; \
		 document.body.appendChild(i); }); \
		 var p = document.createElement('input'); p.type = 'password'; p.id = 'bbClipP'; p.value = 'secret'; document.body.appendChild(p); 0",
	)?;
	// Export: the code lands on the clipboard.
	eval("var a = $('#bbClipA'); a.val('build-1234'); a.select(); document.execCommand('copy'); a.val(''); 0")?;
	ensure(
		clipboard.read_text().as_deref() == Some("build-1234"),
		format!("execCommand('copy') put {:?} on the clipboard", clipboard.read_text()),
	)?;
	// Paste: what another program copied lands in the field, over its selection.
	clipboard.write_text("from élsewhere");
	eval("var b = $('#bbClipB'); b.val('old'); b.select(); document.execCommand('paste'); 0")?;
	ensure(value("bbClipB")? == "from élsewhere", format!("execCommand('paste') left {:?}", value("bbClipB")?))?;
	// The keys: Ctrl+C copies the selection, Ctrl+V pastes at the caret.
	let ctrl = Modifiers { control: true, ..Modifiers::default() };
	let keys = |code: i32| {
		session.host().key(0x11, true, Modifiers::default());
		session.host().key(code, true, ctrl);
		session.host().key(code, false, ctrl);
		session.host().key(0x11, false, Modifiers::default());
	};
	eval("var b = document.getElementById('bbClipB'); b.focus(); b.setSelectionRange(5, 14); 0")?;
	keys(0x43);
	let copied = || clipboard.read_text().as_deref() == Some("élsewhere");
	ensure(session.pump_until(Duration::from_secs(5), |_| copied()), format!("Ctrl+C put {:?} on the clipboard", clipboard.read_text()))?;
	clipboard.write_text("[keyed]");
	eval("var b = document.getElementById('bbClipB'); b.focus(); b.setSelectionRange(4, 4); 0")?;
	keys(0x56);
	let pasted = || value("bbClipB").is_ok_and(|v| v == "from[keyed] élsewhere");
	ensure(session.pump_until(Duration::from_secs(5), |_| pasted()), format!("Ctrl+V left {:?}", value("bbClipB")?))?;
	// A page that takes the copy over: its own text goes.
	eval(
		"var own = function (e) { e.clipboardData.setData('text/plain', \"the page's own\"); e.preventDefault(); }; \
		 document.addEventListener('copy', own); var a = $('#bbClipA'); a.val('ignored'); a.select(); \
		 document.execCommand('copy'); document.removeEventListener('copy', own); 0",
	)?;
	ensure(
		clipboard.read_text().as_deref() == Some("the page's own"),
		format!("a copy the page took over put {:?} on the clipboard", clipboard.read_text()),
	)?;
	eval("var p = document.getElementById('bbClipP'); p.focus(); p.select(); document.execCommand('copy'); 0")?;
	ensure(
		clipboard.read_text().as_deref() == Some("the page's own"),
		format!("a password field copied {:?} out", clipboard.read_text()),
	)?;
	// navigator.clipboard's text calls too.
	eval("navigator.clipboard.writeText('via the API'); 0")?;
	ensure(
		clipboard.read_text().as_deref() == Some("via the API"),
		format!("navigator.clipboard.writeText put {:?} on the clipboard", clipboard.read_text()),
	)?;
	eval("['bbClipA', 'bbClipB', 'bbClipP'].forEach(function (id) { var e = document.getElementById(id); e.parentNode.removeChild(e); }); 0")?;
	Ok(())
}

/// Vanilla's FPS module: not on the page until the game shows it, then on
/// the root screen with the value the game sets, a three-digit one inside its box, and
/// gone again, with its style, once hidden.
fn fps_module_shows_when_asked(session: &UiSession) -> Result<(), String> {
	const MODULE: &str = r#"(function () {
		var box = document.querySelector('.root-screen > .fps-container');
		var style = document.querySelectorAll('style[data-bb-compat=fps]').length;
		if (box === null) return document.querySelectorAll('.fps-container').length + style;
		var b = box.getBoundingClientRect(), v = box.querySelector('.value').getBoundingClientRect();
		return { text: box.querySelector('.value').textContent, height: Math.ceil(v.height),
		         inside: v.right <= b.right && v.bottom <= b.bottom };
	})()"#;
	ensure(session.eval(MODULE)?.as_i64() == Some(0), "an FPS module is on the page before it's shown")?;
	session.show_fps_module(true)?;
	session.set_fps("123");
	session.run_frames(0.1)?;
	let shown = session.eval(MODULE)?;
	let text = shown.get("text").and_then(Json::as_str);
	let height = shown.get("height").and_then(Json::as_i64).unwrap_or(0);
	let inside = shown.get("inside") == Some(&Json::Bool(true));
	ensure(text == Some("123") && height > 0 && inside, format!("the FPS module shows {}", shown.to_text()))?;
	session.show_fps_module(false)?;
	ensure(session.eval(MODULE)?.as_i64() == Some(0), "the FPS module stayed once hidden")?;
	fps_module_follows_the_root_screen(session)
}

/// The FPS display stays on whatever becomes of the root screen, and the game's
/// `setFPS` never reaches a module without its label (vanilla's throws then). Modern
/// Hooks registers a second, never-connected `RootScreen` over the first (the first
/// one's container stays on the page); a root screen can be shown before it has a
/// container and get one later, or lose it again. Each time, the one display shows on
/// the page with the last value.
fn fps_module_follows_the_root_screen(session: &UiSession) -> Result<(), String> {
	// The values of the FPS displays on the page.
	const SHOWN: &str = r#"(function () {
		var all = document.querySelectorAll('.fps-container');
		var out = [];
		for (var i = 0; i < all.length; i++) {
			var v = all[i].querySelector('.value');
			out.push(all[i].closest('.root-screen') !== null && all[i].offsetParent !== null && v ? v.textContent : '(hidden)');
		}
		return out.join(',');
	})()"#;
	let shown = |want: &str, when: &str| -> Result<(), String> {
		let errors: Vec<String> = session
			.log()
			.into_iter()
			.filter(|line| (line.level == "error" || line.level == "bridge") && line.text.contains("FPS"))
			.map(|line| line.text)
			.collect();
		ensure(errors.is_empty(), format!("{when}: page errors: {errors:?}"))?;
		let on_page = session.eval(SHOWN)?;
		ensure(on_page.as_str() == Some(want), format!("{when}: the FPS displays show {}, not {want:?}", on_page.to_text()))
	};
	// Modern Hooks' second root screen, which never gets a container.
	session.eval("window.__bbFirstRoot = Screens.RootScreen; Screens.RootScreen = new RootScreen(); 0")?;
	session.show_fps_module(true)?;
	session.set_fps("45");
	shown("45", "shown with a root screen that has no container")?;
	// That root screen gets a container: the display moves to it, with its value.
	session.eval("Screens.RootScreen.register($('body')); 0")?;
	ensure(
		session.eval("Screens.RootScreen.getContainer().find('.fps-container .value').text()")?.as_str() == Some("45"),
		"the display didn't move to the root screen's own container",
	)?;
	shown("45", "the root screen got a container")?;
	// And loses it: the next value shows on the page's root screen again.
	session.eval("Screens.RootScreen.unregister(); 0")?;
	session.set_fps("56");
	shown("56", "the root screen lost its container")?;
	// The first root screen back.
	session.eval("Screens.RootScreen = window.__bbFirstRoot; delete window.__bbFirstRoot; 0")?;
	session.set_fps("67");
	shown("67", "the first root screen back")?;
	session.show_fps_module(false)?;
	shown("", "hidden")
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

/// Scripts inserted at runtime, as mods' JS loaders insert them: five in one go, the
/// fourth a second copy of the first. Each records itself (with the `data-n` of the
/// element it runs as) and its load event.
const INSERTED_SCRIPTS: &str = r#"(function () {
	window.__order = [];
	function url(name) {
		var code = '__order.push("' + name + '#" + document.currentScript.getAttribute("data-n"));';
		return URL.createObjectURL(new Blob([code], { type: 'text/javascript' }));
	}
	var urls = { a: url('a'), b: url('b'), c: url('c'), d: url('d') };
	window.__insert = function (name, n) {
		var script = document.createElement('script');
		script.src = urls[name];
		script.setAttribute('data-n', n);
		script.onload = function () { __order.push('load:' + name + '#' + n); };
		document.body.appendChild(script);
		return script;
	};
	['a', 'b', 'c', 'a', 'd'].forEach(function (name, i) { __insert(name, i + 1); });
	window.__copyType = document.querySelector('script[data-n="4"]').getAttribute('type');
	return 0;
})()"#;

/// Scripts inserted at runtime run in Coherent's order (probed): in
/// insertion order, except that a copy of a script that is still loading
/// runs right after it, as itself (`document.currentScript`, its own load event); a
/// copy inserted once its URL has loaded runs again.
fn inserted_scripts_run_in_coherents_order(session: &UiSession) -> Result<(), String> {
	let order = |session: &UiSession| -> Result<String, String> {
		Ok(session.eval("__order.join(',')")?.as_str().unwrap_or_default().to_string())
	};
	session.eval(INSERTED_SCRIPTS)?;
	ensure(
		session.eval("__copyType")? == Json::Null,
		"the waiting copy kept a type of its own",
	)?;
	session.pump_until(Duration::from_secs(5), |s| order(s).is_ok_and(|o| o.contains("load:d")));
	let expected = "a#1,load:a#1,a#4,load:a#4,b#2,load:b#2,c#3,load:c#3,d#5,load:d#5";
	let got = order(session)?;
	ensure(got == expected, format!("inserted scripts ran as {got}, not {expected}"))?;

	session.eval("__order = []; __insert('a', 6); 0")?;
	session.pump_until(Duration::from_secs(5), |s| order(s).is_ok_and(|o| o.contains("load:")));
	let got = order(session)?;
	ensure(got == "a#6,load:a#6", format!("a script inserted again ran as {got:?}"))
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
		// The page is painted at the screen's resolution: `scale` pixels a CSS pixel.
		let scale = session.host().scale();
		let edge = |i: usize, limit: u32| {
			let value = fields.get(i).and_then(Json::as_i64).unwrap_or(0);
			((value as f64 * scale).round() as i64).clamp(0, i64::from(limit)) as u32
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

/// At UI scale `scale` the page lays out at the screen size divided by it and is painted
/// at the screen's resolution, as the original zooms it; it still sees a
/// `devicePixelRatio` of 1, as there. A screen point over the element `css` is over the
/// UI, and maps back into the element.
fn drawn_at_the_screens_resolution(
	session: &UiSession,
	scale: f64,
	css: &str,
) -> Result<(), String> {
	let (screen_w, screen_h, _) = session.engine().video().screen();
	let (ui_w, ui_h) = session.engine().video().ui_size();
	let page = session.eval("[window.innerWidth, window.innerHeight, window.devicePixelRatio]")?;
	ensure(
		page.to_text() == format!("[{ui_w},{ui_h},1]"),
		format!(
			"at UI scale {scale} the page sees {}, not [{ui_w},{ui_h},1]",
			page.to_text()
		),
	)?;
	let frame = session.host().frame().ok_or("nothing painted")?;
	let near = |painted: u32, screen: i64| (i64::from(painted) - screen).abs() <= 1;
	ensure(
		near(frame.width, screen_w) && near(frame.height, screen_h),
		format!(
			"at UI scale {scale} the page is painted at {}x{}, not the screen's {screen_w}x{screen_h}",
			frame.width, frame.height
		),
	)?;
	let selector = Json::Str(css.into()).to_text();
	let rect = session.eval(&format!(
		"(function (r) {{ return [r.left, r.top, r.right, r.bottom]; }})(document.querySelector({selector}).getBoundingClientRect())"
	))?;
	let edge = |i: usize| match &rect {
		Json::Array(r) => match r.get(i) {
			Some(Json::Int(n)) => *n as f64,
			Some(Json::Float(f)) => *f,
			_ => 0.0,
		},
		_ => 0.0,
	};
	let (x, y) = (
		(edge(0) + edge(2)) / 2.0 * scale,
		(edge(1) + edge(3)) / 2.0 * scale,
	);
	let (px, py) = session.to_page(x as i64, y as i64);
	ensure(
		f64::from(px) >= edge(0)
			&& f64::from(px) <= edge(2)
			&& f64::from(py) >= edge(1)
			&& f64::from(py) <= edge(3),
		format!(
			"at UI scale {scale} the screen's ({x}, {y}) maps to the page's ({px}, {py}), outside {}",
			rect.to_text()
		),
	)?;
	ensure(
		session.is_over_ui(x as i64, y as i64),
		format!("at UI scale {scale} the screen's ({x}, {y}) isn't over the UI"),
	)
}

/// Bold or italic text in a family with only a regular face (as all of the game's are)
/// draws in that face as it is, as in Coherent (the original's frames: the resource
/// numbers are bold, dialog subtitles italic), where Chromium would embolden or slant
/// it: asking for both paints the same pixels.
fn faces_are_never_faked(session: &UiSession) -> Result<(), String> {
	session.eval(
		"$('<div id=\"bb-faces\">2,400 A large city and trading hub</div>').css({
			position: 'fixed', left: '100px', top: '100px', zIndex: 10000, padding: '4px',
			background: 'black', color: 'white', fontFamily: 'FreeUniversal-Regular', fontSize: '24px'
		}).appendTo(document.body); true",
	)?;
	let paint = |session: &UiSession| -> Result<Vec<u8>, String> {
		session.run_frames(0.3)?;
		let frame = session.host().frame().ok_or("nothing painted")?;
		let rect = session.eval(
			"(function () { var r = document.getElementById('bb-faces').getBoundingClientRect();
			return [Math.floor(r.left), Math.floor(r.top), Math.ceil(r.right), Math.ceil(r.bottom)]; })()",
		)?;
		let scale = session.host().scale();
		let edge = |i: usize, limit: u32| match &rect {
			Json::Array(r) => {
				let value = r.get(i).and_then(Json::as_i64).unwrap_or(0) as f64 * scale;
				(value.round() as i64).clamp(0, i64::from(limit)) as u32
			}
			_ => 0,
		};
		let (left, top) = (edge(0, frame.width), edge(1, frame.height));
		let (right, bottom) = (edge(2, frame.width), edge(3, frame.height));
		Ok((top..bottom)
			.flat_map(|y| {
				frame.bgra[((y * frame.width + left) * 4) as usize
					..((y * frame.width + right) * 4) as usize]
					.to_vec()
			})
			.collect())
	};
	let plain = paint(session)?;
	session.eval("$('#bb-faces').css({ fontWeight: 'bold', fontStyle: 'italic' }); true")?;
	let asked = paint(session);
	session.eval("$('#bb-faces').remove(); true")?;
	let asked = asked?;
	ensure(
		!plain.is_empty() && plain.iter().any(|&b| b != 0 && b != 255),
		"the text didn't paint",
	)?;
	ensure(
		plain == asked,
		"bold italic text in a regular-only face painted differently (faked)",
	)
}

/// Text advances by whole pixels a glyph, as in Coherent (the original's frames: the
/// intro event's lines are the sums of their glyphs' hinted widths), so lines are as
/// wide as there and wrap where they do. Fractional advances made these lines 692.1,
/// 695.6 and 273.6px wide at 16px.
fn text_lays_out_in_whole_pixels(session: &UiSession) -> Result<(), String> {
	let widths = session.eval(
		"(function () {
			var lines = ['maybe they are greedy for quick spoils of war. Most of them will die on the battlefield. But do',
				'not be discouraged. Such is the mercenary life, and the next village will always have new men',
				'eagerly looking for a new start in life.'];
			var widths = [];
			[13, 16, 18, 24].forEach(function (size) {
				lines.forEach(function (line) {
					var span = $('<span/>').text(line).css({ position: 'fixed', whiteSpace: 'nowrap',
						fontFamily: 'FreeUniversal-Regular', fontSize: size + 'px' }).appendTo(document.body);
					widths.push(span[0].getBoundingClientRect().width);
					span.remove();
				});
			});
			return widths.join(' ');
		})()",
	)?;
	let widths = widths.as_str().unwrap_or_default().to_string();
	ensure(
		widths
			.split(' ')
			.all(|w| w.parse::<f64>().is_ok_and(|w| w > 0.0 && w.fract() == 0.0)),
		format!("text advances by fractions of a pixel: line widths {widths}"),
	)
}

/// Where the game discards a scene, the page gives back what it kept (`UiHost::
/// release_memory`), a garbage collection among it: an object nothing holds any more
/// is gone after it. The page holds the object until the release, then lets it go and
/// is told to release at once: V8 collects on its own whenever its heuristics say
/// (more often under load), so an object let go earlier may already be gone, which
/// would say nothing about the release.
fn discards_give_memory_back(session: &UiSession) -> Result<(), String> {
	session.eval("window.__bbHeld = { big: new Array(100000).fill(1) }; window.__bbWeak = new WeakRef(window.__bbHeld); true")?;
	session.run_frames(0.3)?;
	ensure(
		session.eval("delete window.__bbHeld; window.__bbWeak.deref() !== undefined")? == Json::Bool(true),
		"the object went while the page held it",
	)?;
	session.host().release_memory();
	session.run_frames(0.5)?;
	let gone = session.eval("window.__bbWeak.deref() === undefined")?;
	session.eval("delete window.__bbWeak; true")?;
	ensure(gone == Json::Bool(true), "no collection after the release")
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
