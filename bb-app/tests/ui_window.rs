//! The game with its real UI, as `bb` built with `--features ui` plays it, but
//! offscreen (no window): the campaign starts from the menu, and its opening event is
//! drawn over the world and takes the clicks it's under, while the open map passes
//! them to the states. A brother's portrait the page asks for is composed by the
//! renderer. Idle in a town, frames that would show the same are left undrawn.
//! Quitting from the in-game menu (Escape, then Quit) leads back to the main menu, and
//! Escape there ends the game, as in the original. The FPS display is off until
//! asked for, then shows the frame rate. Keys typed into a page text field reach the
//! states too. An idle page rests between frames, its calls and timers on time.
//!
//! Skips if there is no Steam install or no GPU.
//!
//! Run: `cargo test -p bb-app --features ui --test ui_window`
//!
//! A plain `main` (`harness = false`), like bb-ui's tests: the browser host re-runs this
//! binary for its child processes, and starts only once per process.

use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use bb_app::front::Key;
use bb_app::game::Game;
use bb_app::game_cursor::CursorShow;
use bb_engine::harness::STEAM_DATA_DIR;
use bb_assets::Image;
use bb_render::portrait::{PortraitRequest, portrait_size};
use bb_render::{Gpu, RenderTarget, save_png};
use bb_ui::json::Json;
use squirrel::SqTable;
use squirrel_bb::VideoMode;

/// The campaign's map seed (the UI goldens').
const SEED: &str = "bbuitest";

/// Marks the world state: one of the first things a click on the map does there is
/// reset `m.LastAutoAttackPath` to 0 (`world_state.onMouseInput`).
const MARK: &str = "::World.State.m.LastAutoAttackPath = 12345.0;";

/// Whether a click reached the world state since it was marked.
fn clicked(game: &Game) -> Result<bool, String> {
	let engine = game.front().engine();
	engine.run_script(
		"::__clicked <- ::World.State.m.LastAutoAttackPath != 12345.0;",
		"clicked",
	)?;
	SqTable::root(engine.vm())
		.get::<bool>("__clicked")
		.ok_or_else(|| "no answer".to_string())
}

/// What `UI.isMouseOver()` answers.
fn mouse_over_ui(game: &Game) -> Result<bool, String> {
	let engine = game.front().engine();
	engine.run_script("::__over <- ::UI.isMouseOver();", "over")?;
	SqTable::root(engine.vm())
		.get::<bool>("__over")
		.ok_or_else(|| "no answer".to_string())
}

fn pixel(rgba: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
	let i = ((y * width + x) * 4) as usize;
	rgba[i..i + 4].try_into().unwrap()
}

/// Draw the game into `target` until `ready` holds for the frame: atlases load on
/// demand, so the first frames may lack them.
fn draw_until(game: &mut Game, gpu: &Gpu, target: &RenderTarget, ready: impl Fn(&Image) -> bool) -> Result<Image, String> {
	for _ in 0..100 {
		game.step(1000.0 / 60.0)?;
		// A frame whose sprites are all drawn (none waiting for its frame to load).
		let whole = game.draw(gpu, target).waiting == 0;
		let frame = target.read(gpu)?;
		if whole && ready(&frame) {
			return Ok(frame);
		}
		std::thread::sleep(Duration::from_millis(50));
	}
	Err("the frame never got ready".into())
}

/// The first brother's portrait, as the page loads it (`Path.PROCEDURAL` and his
/// `getImagePath()`), arrives the size the compositor makes it.
fn portrait_reaches_the_page(game: &mut Game, gpu: &Gpu, target: &RenderTarget) -> Result<(), String> {
	let engine = game.front().engine();
	engine.run_script(
		"::__portrait <- ::World.getPlayerRoster().getAll()[0].getImagePath();",
		"portrait path",
	)?;
	let path = SqTable::root(engine.vm())
		.get::<String>("__portrait")
		.ok_or("no image path")?;
	let request = PortraitRequest::parse(&path).ok_or_else(|| format!("not a portrait: {path}"))?;
	let portrait = engine.portrait(request.id).ok_or("no such entity")?;
	let expected = engine
		.sprites()
		.with(|registry| portrait_size(&portrait, &request.hidden, registry));
	let mut url = String::new();
	Json::Str(format!("coui://procedural/{path}")).write(&mut url);
	game.front().session().eval(&format!(
		"(function () {{
	var image = new Image();
	window.__portrait = null;
	image.onload = function () {{ window.__portrait = [image.naturalWidth, image.naturalHeight]; }};
	image.src = {url};
}})()"
	))?;
	// Drawing a frame answers the page's procedural images.
	for _ in 0..100 {
		game.step(1000.0 / 60.0)?;
		game.draw(gpu, target);
		let loaded = game.front().session().eval("window.__portrait")?;
		if let Json::Array(size) = loaded {
			let size: Vec<i64> = size.iter().filter_map(|n| n.as_i64()).collect();
			if size != [i64::from(expected.0), i64::from(expected.1)] || expected.0 == 0 {
				return Err(format!("{path} loaded at {size:?}, composed at {expected:?}"));
			}
			println!("  portrait {path}: {}x{}", expected.0, expected.1);
			return Ok(());
		}
		std::thread::sleep(Duration::from_millis(20));
	}
	Err(format!("{path} never loaded"))
}

/// The game's cursor (on the open map the scripts' boot, centred on the mouse) is drawn
/// at the mouse, at `at`: its pixels change when the scripts switch to the system's
/// cursor, far more than a patch of the map elsewhere.
fn cursor_is_drawn_at_the_mouse(game: &mut Game, gpu: &Gpu, target: &RenderTarget, at: (i32, i32)) -> Result<(), String> {
	let drawn = draw_until(game, gpu, target, |_| true)?;
	game.front().engine().run_script("::UI.setCursorHardware(true);", "hardware")?;
	let system = draw_until(game, gpu, target, |_| true)?;
	game.front().engine().run_script("::UI.setCursorHardware(false);", "software")?;
	let differing = |x0: i32, y0: i32| {
		(-16..16)
			.flat_map(|dy| (-16..16).map(move |dx| (x0 + dx, y0 + dy)))
			.filter(|&(x, y)| x >= 0 && y >= 0 && (x as u32) < drawn.width && (y as u32) < drawn.height)
			.filter(|&(x, y)| {
				let (a, b) = (pixel(&drawn.rgba, drawn.width, x as u32, y as u32), pixel(&system.rgba, system.width, x as u32, y as u32));
				a.iter().zip(b).any(|(p, q)| p.abs_diff(q) > 40)
			})
			.count()
	};
	let (cursor, elsewhere) = (differing(at.0, at.1), differing(at.0 + 200, at.1 + 100));
	if cursor < 60 || elsewhere > 20 {
		return Err(format!("the cursor isn't drawn at {at:?}: {cursor} pixels differ there, {elsewhere} elsewhere"));
	}
	println!("  cursor: {cursor} pixels at the mouse, {elsewhere} elsewhere");
	Ok(())
}

/// Step and draw until `n` frames in a row are left undrawn.
fn settle(game: &mut Game, gpu: &Gpu, target: &RenderTarget, n: u32) -> Result<(), String> {
	let mut undrawn = 0;
	for _ in 0..600 {
		game.step(1000.0 / 60.0)?;
		undrawn = if game.draw_if_changed(gpu, target).is_none() { undrawn + 1 } else { 0 };
		if undrawn == n {
			return Ok(());
		}
		std::thread::sleep(Duration::from_millis(10));
	}
	Err(format!("no {n} frames in a row were left undrawn"))
}

/// With the game's cursor shown as the system's (the players' default), the system
/// shows the scripts' sprite, and a mouse move over the still town draws nothing. When
/// the scripts put the cursor elsewhere than the mouse, the game draws it there.
fn system_cursor_moves_draw_nothing(game: &mut Game, gpu: &Gpu, target: &RenderTarget) -> Result<(), String> {
	game.use_system_cursor(true);
	let show = game.cursor_show();
	let CursorShow::Sprite { brush, frame: 0 } = &show else {
		return Err(format!("the system cursor on, {show:?} shows"));
	};
	let pictures = game.cursor_pictures(brush).ok_or("no pictures")?;
	if pictures.is_empty() || pictures[0].width < 16 {
		return Err(format!("{brush}'s pictures: {} of {} px", pictures.len(), pictures.first().map_or(0, |p| p.width)));
	}
	settle(game, gpu, target, 5)?;
	for (x, y) in [(12, 12), (14, 13), (16, 15)] {
		game.mouse_moved(x, y)?;
		game.step(1000.0 / 60.0)?;
		if game.draw_if_changed(gpu, target).is_some() {
			return Err(format!("the system's cursor moved to {x}, {y}, and a frame was drawn"));
		}
	}
	game.front().engine().run_script("::UI.setCursorPosition(300, 300);", "move the cursor")?;
	if game.cursor_show() != CursorShow::Drawn {
		return Err("the scripts moved the cursor away from the mouse, but it isn't drawn".into());
	}
	game.mouse_moved(18, 16)?;
	if game.cursor_show() != show {
		return Err(format!("back at the mouse, {:?} shows", game.cursor_show()));
	}
	game.use_system_cursor(false);
	println!("  the system's cursor: {brush}, moves draw nothing");
	Ok(())
}

/// Idling in a town (the world paused behind the town's dialog), frames come to show
/// what the last did, and are left undrawn: the frame left up is pixel for pixel what
/// drawing it would draw. A move of the mouse draws again (the cursor moved).
fn unchanged_frames_are_left_undrawn(game: &mut Game, gpu: &Gpu, target: &RenderTarget) -> Result<(), String> {
	game.front().engine().run_script(
		r#"::World.State.m.IsAutosaving = false;
		local state = ::World.State;
		local tile = state.getPlayer().getTile();
		local nearest = null;
		foreach (s in ::World.EntityManager.getSettlements())
			if (s.isEnterable() && (nearest == null || s.getTile().getDistanceTo(tile) < nearest.getTile().getDistanceTo(tile)))
				nearest = s;
		state.enterLocation(nearest);"#,
		"enter a town",
	)?;
	let mut undrawn = 0;
	for _ in 0..600 {
		game.step(1000.0 / 60.0)?;
		if game.draw_if_changed(gpu, target).is_none() {
			undrawn += 1;
			if undrawn == 10 {
				break;
			}
		}
		std::thread::sleep(Duration::from_millis(10));
	}
	if undrawn < 10 {
		return Err(format!("idling in a town, only {undrawn} of 600 frames were left undrawn"));
	}
	let left = target.read(gpu)?;
	game.draw(gpu, target);
	let drawn = target.read(gpu)?;
	let differ = left.rgba.chunks(4).zip(drawn.rgba.chunks(4)).filter(|(a, b)| a != b).count();
	if differ > 0 {
		let out = std::env::temp_dir().join("bb-app-ui_window-undrawn.png");
		save_png(out.to_string_lossy().as_ref(), &left)?;
		return Err(format!("the frame left up differs from the one drawn in {differ} pixels ({})", out.display()));
	}
	// The scene isn't built to tell (`Engine::world_scene_key`): a script setting
	// something on an entity draws again.
	let settled = (0..600).any(|_| {
		game.step(1000.0 / 60.0).is_ok() && game.draw_if_changed(gpu, target).is_none()
	});
	if !settled {
		return Err("idling in a town, frames were no longer left undrawn".into());
	}
	game.front().engine().run_script(
		"::World.State.getPlayer().getSprite(\"body\").Color = ::createColor(\"#ff000080\");",
		"tint the player",
	)?;
	game.step(1000.0 / 60.0)?;
	if game.draw_if_changed(gpu, target).is_none() {
		return Err("a sprite was tinted, but the frame was left undrawn".into());
	}
	game.mouse_moved(10, 10)?;
	game.step(1000.0 / 60.0)?;
	if game.draw_if_changed(gpu, target).is_none() {
		return Err("the cursor moved, but the frame was left undrawn".into());
	}
	println!("  idle in a town: frames left undrawn, the same as drawn");
	system_cursor_moves_draw_nothing(game, gpu, target)?;
	an_idle_page_rests(game, gpu, target)?;
	game.front().engine().run_script(
		"::World.State.town_screen_main_dialog_module_onLeaveButtonClicked();",
		"leave the town",
	)?;
	play(game, gpu, target, 1.0)
}

/// A 60th of a second of the game as the window runs it: stepped, drawn if changed, then
/// the rest of the frame's time waited out.
fn frame_in_real_time(game: &mut Game, gpu: &Gpu, target: &RenderTarget) -> Result<(), String> {
	let start = Instant::now();
	game.step(1000.0 / 60.0)?;
	game.draw_if_changed(gpu, target);
	std::thread::sleep(Duration::from_micros(16_667).saturating_sub(start.elapsed()));
	Ok(())
}

/// Idle in a town, the page comes to rest: parked across the frames, none of its frames
/// begun. The scripts' calls still reach it at once, and timers set while it rests fire
/// on time, however they were set, the page resting while they wait.
fn an_idle_page_rests(game: &mut Game, gpu: &Gpu, target: &RenderTarget) -> Result<(), String> {
	let resting = |game: &Game| game.front().session().host().is_resting();
	let mut frames = 0;
	while !resting(game) {
		frames += 1;
		if frames > 600 {
			return Err("idle in a town, the page never came to rest".into());
		}
		frame_in_real_time(game, gpu, target)?;
	}
	let rested = game.front().page_rested();
	for _ in 0..30 {
		frame_in_real_time(game, gpu, target)?;
	}
	if game.front().page_rested() < rested + 30 || !resting(game) {
		return Err("idle in a town, the page didn't stay at rest".into());
	}
	let sum = game.front().session().eval("6 * 7")?;
	if sum.as_i64() != Some(42) {
		return Err(format!("a call into the resting page answered {sum:?}"));
	}
	for (context, name, code) in TIMER_CONTEXTS {
		a_timer_fires_on_time(game, gpu, target, context, name, code)?;
	}
	Ok(())
}

/// How long the timers [`TIMER_CONTEXTS`] set wait.
const TIMER_MS: f64 = 300.0;

/// Ways a timer gets set while the page rests, as the code a call into the page runs:
/// `__bbArm(name)` sets a [`TIMER_MS`] timer that records, under `name` (the middle
/// field), how long after it was set it fired.
const TIMER_CONTEXTS: [(&str, &str, &str); 6] = [
	("the game's call", "call", "__bbArm('call');"),
	("a promise's callback", "promise", "Promise.resolve().then(function () { __bbArm('promise'); });"),
	("another timer", "timer", "setTimeout(function () { __bbArm('timer'); }, 100);"),
	("an animation frame", "frame", "requestAnimationFrame(function () { __bbArm('frame'); });"),
	(
		"a load",
		"load",
		"var x = new XMLHttpRequest(); x.onload = function () { __bbArm('load'); }; x.open('GET', 'coui://ui/main.html'); x.send();",
	),
	(
		"an interval",
		"interval",
		"(function (set, n) { var h = setInterval(function () { if (++n === 3) { clearInterval(h); __bbTimers.interval = performance.now() - set; } }, 100); })(performance.now(), 0);",
	),
];

/// Idle in a town, with the page resting, a call into it runs `code`, which sets a
/// timer in `context` (see [`TIMER_CONTEXTS`]): the timer fires in the frame its due
/// time falls in, as it did without resting, and the page rests while it waits.
fn a_timer_fires_on_time(
	game: &mut Game,
	gpu: &Gpu,
	target: &RenderTarget,
	context: &str,
	name: &str,
	code: &str,
) -> Result<(), String> {
	let resting = |game: &Game| game.front().session().host().is_resting();
	let mut frames = 0;
	while !resting(game) {
		frames += 1;
		if frames > 600 {
			return Err(format!("before a timer set from {context}, the page never came back to rest"));
		}
		frame_in_real_time(game, gpu, target)?;
	}
	game.front().session().eval(&format!(
		"window.__bbTimers = window.__bbTimers || {{}}; window.__bbTimers.{name} = null;
		window.__bbArm = window.__bbArm || function (name) {{
			var set = performance.now();
			setTimeout(function () {{ window.__bbTimers[name] = performance.now() - set; }}, {TIMER_MS});
		}};
		{code} 0"
	))?;
	let mut rested = 0;
	let start = Instant::now();
	let after = loop {
		frame_in_real_time(game, gpu, target)?;
		if resting(game) {
			rested += 1;
		}
		// Asked with the page parked, as the scripts' calls reach it: a call into a page
		// on its turn would give it the next turn too.
		game.front().session().park();
		let fired = game.front().session().eval(&format!("window.__bbTimers.{name}"))?;
		if let Some(after) = fired.as_f64() {
			break after;
		}
		if start.elapsed() > Duration::from_secs(2) {
			return Err(format!("a {TIMER_MS} ms timer set from {context} never fired ({fired:?})"));
		}
	};
	// It fires in the frame its due time falls in, as it did without resting: by the
	// end of the turn after it (two frames), with some leeway.
	if !(TIMER_MS..=TIMER_MS + 2.0 * 16.7 + 10.0).contains(&after) {
		return Err(format!("a {TIMER_MS} ms timer set from {context} fired after {after:.1} ms"));
	}
	if rested < 5 {
		return Err(format!("the page rested {rested} frames while a timer set from {context} waited"));
	}
	println!("  idle, the page rests: a {TIMER_MS} ms timer set from {context} fired after {after:.1} ms");
	Ok(())
}

/// `seconds` of 60th-of-a-second frames, stepped and drawn.
fn play(game: &mut Game, gpu: &Gpu, target: &RenderTarget, seconds: f64) -> Result<(), String> {
	for _ in 0..(seconds * 60.0) as u32 {
		game.step(1000.0 / 60.0)?;
		game.draw(gpu, target);
		std::thread::sleep(Duration::from_millis(5));
	}
	Ok(())
}

/// Escape, as a player presses it: down, and up a moment later.
fn escape(game: &mut Game, gpu: &Gpu, target: &RenderTarget) -> Result<(), String> {
	for down in [true, false] {
		let key = Key {
			engine: Some(41),
			text: None,
			down,
			repeat: false,
		};
		game.key(&key)?;
		play(game, gpu, target, 0.2)?;
	}
	Ok(())
}

/// The in-game menu's Quit, as a player quits (Escape, then a click on Quit): the main
/// menu comes back, and the game goes on. Escape there finishes the scripts' last state,
/// which ends the original, and `bb` exits.
fn quits_from_the_in_game_menu(game: &mut Game, gpu: &Gpu, target: &RenderTarget) -> Result<(), String> {
	escape(game, gpu, target)?;
	let (qx, qy) = game.front().session().locate_when_ready("Quit", Duration::from_secs(30))?;
	game.mouse_moved(i64::from(qx), i64::from(qy))?;
	game.mouse_button(1, true)?;
	game.mouse_button(1, false)?;
	let deadline = std::time::Instant::now() + Duration::from_secs(120);
	loop {
		play(game, gpu, target, 0.2)?;
		let names = game.front().engine().root_state().state_names();
		if names == ["MainMenuState"] && game.front().session().visible_buttons()?.iter().any(|b| b == "New Campaign") {
			break;
		}
		if std::time::Instant::now() > deadline {
			return Err(format!("Quit didn't lead back to the main menu: {names:?}"));
		}
	}
	play(game, gpu, target, 2.0)?;
	// The FPS display, left on over the world, shows on the main menu too.
	match fps_shown(game)? {
		Some(shown) if shown.parse::<u32>().is_ok_and(|fps| fps > 0) => {}
		shown => return Err(format!("back on the main menu, the FPS display shows {shown:?}")),
	}
	if game.quit() {
		return Err("the game ended at the main menu".into());
	}
	// As a player waits: until the menu takes keys (the loading screen has faded out;
	// it fades by the page's clock, so how many frames that takes depends on how fast
	// they come).
	let deadline = std::time::Instant::now() + Duration::from_secs(30);
	while bb_engine::tactical_player::eval_i64(game.front().engine(), "::RootState.get(\"MainMenuState\").isKeyInputPermitted() ? 1 : 0")? != Some(1) {
		if std::time::Instant::now() > deadline {
			return Err("the main menu never took keys again".into());
		}
		play(game, gpu, target, 0.2)?;
	}
	escape(game, gpu, target)?;
	play(game, gpu, target, 1.0)?;
	if !game.quit() {
		return Err("Escape on the main menu didn't end the game".into());
	}
	Ok(())
}

/// Whether the world's character screen is up, done animating in (`true`), or gone,
/// done animating out (`false`); `None` while it moves.
fn character_screen_shown(game: &Game) -> Result<Option<bool>, String> {
	let engine = game.front().engine();
	engine.run_script(
		"local s = ::World.State.m.CharacterScreen;
		::__characterScreen <- s.isAnimating() ? null : s.isVisible();",
		"character screen",
	)?;
	Ok(SqTable::root(engine.vm()).get::<bool>("__characterScreen"))
}

/// Keys typed into a text field of the page reach the states too, as in the original,
/// whose UI never keeps a key from them (MSU's `blockSQInput` setting describes it:
/// "writing a 'c' would open the stash screen"): a `c` typed into a focused field goes
/// into it and toggles the character screen, and again closes it. The field is a
/// stand-in for vanilla's name fields, focused as a click into one focuses it.
fn keys_reach_the_states_while_typing(game: &mut Game, gpu: &Gpu, target: &RenderTarget) -> Result<(), String> {
	let session = game.front().session();
	session.eval(
		"(function () {
			var field = document.createElement('input');
			field.id = 'bbTypingTest';
			field.type = 'text';
			field.setAttribute('style', 'position:fixed;left:-500px;top:0;');
			document.body.appendChild(field);
			field.focus();
			return document.activeElement === field;
		})()",
	)?;
	let typed = |game: &Game| -> Result<String, String> {
		let value = game.front().session().eval("document.getElementById('bbTypingTest').value")?;
		Ok(value.as_str().unwrap_or_default().to_string())
	};
	for (round, shown) in [(1, true), (2, false)] {
		for down in [true, false] {
			let key = Key {
				engine: Some(13),
				text: down.then(|| "c".to_string()),
				down,
				repeat: false,
			};
			game.key(&key)?;
			play(game, gpu, target, 0.1)?;
		}
		let deadline = std::time::Instant::now() + Duration::from_secs(20);
		while character_screen_shown(game)? != Some(shown) {
			if std::time::Instant::now() > deadline {
				return Err(format!(
					"a `c` typed into a text field didn't {} the character screen (the field has {:?})",
					if shown { "open" } else { "close" },
					typed(game)?
				));
			}
			play(game, gpu, target, 0.2)?;
		}
		let expected = "c".repeat(round);
		if typed(game)? != expected {
			return Err(format!("the text field has {:?}, not {expected:?}", typed(game)?));
		}
	}
	game.front()
		.session()
		.eval("document.body.removeChild(document.getElementById('bbTypingTest')); true")?;
	Ok(())
}

/// What vanilla's FPS module shows, if it's on the page.
fn fps_shown(game: &Game) -> Result<Option<String>, String> {
	let value = game
		.front()
		.session()
		.eval("(function () { var v = document.querySelector('.fps-container .value'); return v && v.textContent; })()")?;
	Ok(value.as_str().map(str::to_string))
}

/// The FPS display: once shown, vanilla's FPS module shows the frames the
/// game runs a second, here about 20 (frames 50 ms apart); hidden, it's gone. It's left
/// on after.
fn shows_the_frame_rate_when_asked(game: &mut Game, gpu: &Gpu, target: &RenderTarget) -> Result<(), String> {
	game.show_fps(true)?;
	let start = std::time::Instant::now();
	let mut frames = 0u32;
	while start.elapsed() < Duration::from_secs(2) {
		game.step(50.0)?;
		game.draw(gpu, target);
		frames += 1;
		std::thread::sleep(Duration::from_millis(50));
	}
	let rate = f64::from(frames) / start.elapsed().as_secs_f64();
	let shown = fps_shown(game)?.ok_or("no FPS display once shown")?;
	let value: f64 = shown.parse().map_err(|_| format!("the FPS display shows {shown:?}"))?;
	if (value - rate).abs() > rate * 0.3 {
		return Err(format!("the FPS display shows {value}, the frames ran at {rate:.1} a second"));
	}
	println!("  the FPS display showed {value} (frames at {rate:.1} a second)");
	game.show_fps(false)?;
	if fps_shown(game)?.is_some() {
		return Err("the FPS display stayed once hidden".into());
	}
	// On again, for the way back to the main menu (`quits_from_the_in_game_menu`).
	game.show_fps(true)
}

fn check(gpu: &Gpu) -> Result<(), String> {
	// Silent: tests never play on the device.
	let mut game = Game::boot(gpu, Path::new(STEAM_DATA_DIR), SEED, false, None, VideoMode::DEFAULT, None)?;
	let session = game.front().session();
	// The opening event, over the world. The page is at UI scale 1: its pixels are
	// the screen's.
	let (bx, by) = session.locate_when_ready("Huzzah!", Duration::from_secs(180))?;
	session.run_frames(1.0)?;
	if fps_shown(&game)?.is_some() {
		return Err("the FPS display shows by default".into());
	}

	// Drawn over the world: the event's opaque button shows the page; its dimmed
	// backdrop lets the world show through.
	let (width, height) = game.screen();
	let target = RenderTarget::new(gpu, width, height);
	let frame = draw_until(&mut game, gpu, &target, |f| {
		let [r, g, b, _] = pixel(&f.rgba, f.width, 10, 10);
		[r, g, b] != [0, 0, 0]
	})?;
	let out = std::env::temp_dir().join("bb-app-ui_window.png");
	save_png(out.to_string_lossy().as_ref(), &frame)?;
	println!("  composited frame: {}", out.display());
	let (page_width, page) = game
		.front()
		.session()
		.host()
		.frame()
		.map(|f| (f.width, f.bgra))
		.ok_or("the page never painted")?;
	let at_button = pixel(&page, page_width, bx as u32, by as u32);
	if at_button[3] != 255 {
		return Err(format!("the button isn't opaque in the page: {at_button:?}"));
	}
	let [b, g, r, _] = at_button;
	let shown = pixel(&frame.rgba, frame.width, bx as u32, by as u32);
	if shown[..3] != [r, g, b] {
		return Err(format!("the frame shows {shown:?} under the button, the page {at_button:?}"));
	}

	// Over the event: the UI's. A click there goes to the page and closes the event.
	game.mouse_moved(i64::from(bx), i64::from(by))?;
	if !mouse_over_ui(&game)? {
		return Err("UI.isMouseOver() is false over the event".into());
	}
	game.front().engine().run_script(MARK, "mark")?;
	game.mouse_button(1, true)?;
	game.mouse_button(1, false)?;
	let session = game.front().session();
	let deadline = std::time::Instant::now() + Duration::from_secs(30);
	while session.shows_text("Huzzah!")? {
		if std::time::Instant::now() >= deadline {
			return Err("the event stayed after its button was clicked".into());
		}
		session.run_frames(0.1)?;
	}
	if clicked(&game)? {
		return Err("the states got the event's click too".into());
	}
	portrait_reaches_the_page(&mut game, gpu, &target)?;

	// The open map: a spot where the page is transparent now, away from the top bar.
	let session = game.front().session();
	session.run_frames(1.0)?;
	let host = session.host();
	let (mx, my) = (0..20)
		.flat_map(|i| (0..20).map(move |j| (width as i32 * (2 + i) / 24, height as i32 * (4 + j) / 28)))
		.find(|&(x, y)| host.alpha_at(x, y) == 0)
		.ok_or_else(|| {
			let corner = host.frame().map(|f| pixel(&f.bgra, f.width, 10, 10));
			format!("no transparent spot on the page (its corner is {corner:?})")
		})?;
	// The world shows there (the renderer's own alpha isn't always opaque).
	draw_until(&mut game, gpu, &target, |f| {
		let [r, g, b, _] = pixel(&f.rgba, f.width, mx as u32, my as u32);
		[r, g, b] != [0, 0, 0]
	})?;
	// Not the UI's, and clicks go to the states.
	game.mouse_moved(i64::from(mx), i64::from(my))?;
	if mouse_over_ui(&game)? {
		return Err(format!("UI.isMouseOver() is true over the map at {mx}, {my}"));
	}
	cursor_is_drawn_at_the_mouse(&mut game, gpu, &target, (mx, my))?;
	game.front().engine().run_script(MARK, "mark")?;
	game.mouse_button(1, true)?;
	// Our world map can't move the party yet, so the click may stop with an error
	// after it has reached the state.
	let release = game.mouse_button(1, false);
	if !clicked(&game)? {
		return Err(format!("the map's click didn't reach the states ({release:?})"));
	}
	keys_reach_the_states_while_typing(&mut game, gpu, &target)?;
	game.mouse_moved(0, 0)?;
	unchanged_frames_are_left_undrawn(&mut game, gpu, &target)?;
	shows_the_frame_rate_when_asked(&mut game, gpu, &target)?;
	quits_from_the_in_game_menu(&mut game, gpu, &target)
}

fn main() -> ExitCode {
	if let Some(code) = bb_ui::host::run_subprocess() {
		return ExitCode::from(code.clamp(0, 255) as u8);
	}
	// Chromium paints the page through a display connection: a headless gamescope's.
	bb_ui::host::with_display(run_main)
}

fn run_main() -> ExitCode {
	let name = "the_ui_draws_over_the_game_and_takes_its_input";
	if !Path::new(STEAM_DATA_DIR).exists() {
		println!("test {name} ... skipped ({STEAM_DATA_DIR} not present)");
		return ExitCode::SUCCESS;
	}
	let gpu = match Gpu::headless_with(bb_app::front::GPU_FEATURES) {
		Ok(gpu) => gpu,
		Err(e) => {
			println!("test {name} ... skipped (no GPU: {e})");
			return ExitCode::SUCCESS;
		}
	};
	match check(&gpu) {
		Ok(()) => {
			println!("test {name} ... ok");
			ExitCode::SUCCESS
		}
		Err(e) => {
			println!("test {name} ... FAILED\n{e}");
			ExitCode::FAILURE
		}
	}
}
