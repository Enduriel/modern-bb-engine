//! The game with its real UI, as `bb` built with `--features ui` plays it, but
//! offscreen (no window): the campaign starts from the menu, and its opening event is
//! drawn over the world and takes the clicks it's under, while the open map passes
//! them to the states. A brother's portrait the page asks for is composed by the
//! renderer.
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

use bb_app::game::Game;
use bb_engine::harness::STEAM_DATA_DIR;
use bb_assets::Image;
use bb_render::portrait::{PortraitRequest, portrait_size};
use bb_render::{Gpu, RenderTarget, save_png};
use bb_ui::json::Json;
use squirrel::SqTable;

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

fn check(gpu: &Gpu) -> Result<(), String> {
	let mut game = Game::boot(gpu, Path::new(STEAM_DATA_DIR), SEED)?;
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
	game.front().engine().run_script(MARK, "mark")?;
	game.mouse_button(1, true)?;
	// Our world map can't move the party yet, so the click may stop with an error
	// after it has reached the state.
	let release = game.mouse_button(1, false);
	if !clicked(&game)? {
		return Err(format!("the map's click didn't reach the states ({release:?})"));
	}
	Ok(())
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
