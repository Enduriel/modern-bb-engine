//! The game with its real UI, as `bb` built with `--features ui` plays it, but
//! offscreen (no window): the campaign starts from the menu, and its opening event is
//! drawn over the world and takes the clicks it's under, while the open map passes
//! them to the states. A brother's portrait the page asks for is composed by the
//! renderer. Quitting from the in-game menu (Escape, then Quit) leads back to the main
//! menu, and Escape there ends the game, as in the original.
//!
//! Skips if there is no Steam install or no GPU.
//!
//! Run: `cargo test -p bb-app --features ui --test ui_window`
//!
//! A plain `main` (`harness = false`), like bb-ui's tests: the browser host re-runs this
//! binary for its child processes, and starts only once per process.

use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

use bb_app::front::Key;
use bb_app::game::Game;
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
		game.draw(gpu, target);
		let frame = target.read(gpu)?;
		if ready(&frame) {
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
			windows: Some(0x1B),
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
	if game.quit() {
		return Err("the game ended at the main menu".into());
	}
	escape(game, gpu, target)?;
	play(game, gpu, target, 1.0)?;
	if !game.quit() {
		return Err("Escape on the main menu didn't end the game".into());
	}
	Ok(())
}

fn check(gpu: &Gpu) -> Result<(), String> {
	// Silent: tests never play on the device.
	let mut game = Game::boot(gpu, Path::new(STEAM_DATA_DIR), SEED, false, None, VideoMode::DEFAULT, None)?;
	let session = game.front().session();
	// The opening event, over the world. The page is at UI scale 1: its pixels are
	// the screen's.
	let (bx, by) = session.locate_when_ready("Huzzah!", Duration::from_secs(180))?;
	session.run_frames(1.0)?;

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
		.with_frame(|f| (f.width, f.bgra.clone()))
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
			let corner = host.with_frame(|f| pixel(&f.bgra, f.width, 10, 10));
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
	game.mouse_moved(0, 0)?;
	quits_from_the_in_game_menu(&mut game, gpu, &target)
}

fn main() -> ExitCode {
	if let Some(code) = bb_ui::host::run_subprocess() {
		return ExitCode::from(code.clamp(0, 255) as u8);
	}
	let name = "the_ui_draws_over_the_game_and_takes_its_input";
	if !Path::new(STEAM_DATA_DIR).exists() {
		println!("test {name} ... skipped ({STEAM_DATA_DIR} not present)");
		return ExitCode::SUCCESS;
	}
	let gpu = match Gpu::headless() {
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
