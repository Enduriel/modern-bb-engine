//! How often a battle's frame shows what the last did, and what changes when it
//! doesn't: the battle `render_battle` starts, on the player's first turn with nobody
//! acting, stepped at 60 frames a second; each frame's scene (what `bb` compares to
//! leave a frame undrawn, see `bb_app::shown`) against the last.
//!
//! ```text
//! cargo run --release -p bb-render --example battle_still -- [FRAMES] [--mods] [--rain] [--fog] [--pause] [--menu] [--hz N] [--steps]
//! ```
//!
//! `--rain` builds a storm's rain over the battle first, `--fog` leaves the fog of war
//! on (the screenshots' camera turns it off), `--pause` pauses the battle (as its menus
//! do), `--menu` opens its menu. `--hz` steps the frames at N a second (60), `--steps`
//! draws the battle in steps as `bb` does (`squirrel_bb::scene::steps`, a frame pixel to
//! a pixel of the game's screen).
//!
//! `GROUND=1` also times building the ground alone (the tiles without their details,
//! nothing on them); `SHOW_POS=1` prints each entity that moved between frames.

use std::collections::BTreeMap;
use std::path::Path;

use bb_engine::harness::{self, STEAM_DATA_DIR};
use squirrel_bb::scene::tactical::TacticalScene;

fn main() {
	let mut args: Vec<String> = std::env::args().skip(1).collect();
	let mods = take_flag(&mut args, "--mods");
	let rain = take_flag(&mut args, "--rain");
	let fog = take_flag(&mut args, "--fog");
	let pause = take_flag(&mut args, "--pause");
	let menu = take_flag(&mut args, "--menu");
	let steps = take_flag(&mut args, "--steps");
	let hz: f64 = take_value(&mut args, "--hz").map_or(60.0, |n| n.parse().expect("--hz N"));
	let frames: usize = args.first().map_or(600, |n| n.parse().expect("FRAMES"));

	let dir = Path::new(STEAM_DATA_DIR);
	let engine = if mods { harness::open_install_with_mods(dir) } else { harness::open_install(dir) }.expect("open install");
	harness::boot_menu(&engine).expect("menu");
	harness::prepare_new_campaign(&engine, "rust-engine-test").expect("prepare");
	harness::start_new_campaign(&engine).expect("start");
	for _ in 0..40 {
		engine.frame(16.0).expect("frame");
	}
	start_battle(&engine, fog);
	if rain {
		engine
			.run_script(
				"local w = ::Tactical.getWeather(); local r = w.createRainSettings(); r.MinDrops = 150; r.MaxDrops = 150; \
				 r.NumSplats = 50; r.MinVelocity = 400.0; r.MaxVelocity = 500.0; r.MinAlpha = 1.0; r.MaxAlpha = 1.0; \
				 r.MinScale = 0.75; r.MaxScale = 1.0; w.buildRain(r);",
				"rain",
			)
			.expect("rain");
	}
	if pause {
		engine.run_script("::Tactical.State.setPause(true);", "pause").expect("pause");
	}
	if menu {
		engine.run_script("::Tactical.State.toggleMenuScreen();", "menu").expect("menu");
	}
	if steps {
		engine.set_scene_steps(Some(1.0));
	}
	let mut doublings: BTreeMap<u32, usize> = BTreeMap::new();
	let mut last = engine.tactical_scene();
	// The quads `bb` would draw at 2560x1358 (zoom 1 on the map's centre): how many stay
	// the same, place by place, from frame to frame.
	let view = |scene: &TacticalScene| bb_render::world::WorldView {
		center: [scene.camera.pos.0 as f32, scene.camera.pos.1 as f32],
		zoom: 1.0 / scene.camera.zoom as f32,
		width: 2560,
		height: 1358,
	};
	let quads_of = |scene: &TacticalScene| {
		engine.sprites().with(|registry| bb_render::tactical::tactical_quads(scene, registry, &view(scene)).quads)
	};
	let mut last_quads = quads_of(&last);
	let (mut quads, mut same_quads, mut same_prefix) = (0, 0, 0);
	let mut build = std::time::Duration::ZERO;
	let (mut ground_build, mut ground_quads) = (std::time::Duration::ZERO, 0);
	let mut changed: BTreeMap<String, usize> = BTreeMap::new();
	let mut still = 0;
	let mut runs = Vec::new();
	let mut run = 0;
	for _ in 0..frames {
		engine.frame(1000.0 / hz).expect("frame");
		engine.pump_ui().expect("ui");
		let scene = engine.tactical_scene();
		if let Some(k) = engine.scene_step_doublings() {
			*doublings.entry(k).or_default() += 1;
		}
		let started = std::time::Instant::now();
		let now = quads_of(&scene);
		build += started.elapsed();
		if std::env::var_os("GROUND").is_some() {
			let mut ground = scene.clone();
			for t in &mut ground.tiles {
				t.details.clear();
			}
			ground.entities.clear();
			ground.clouds.clear();
			ground.rain.clear();
			ground.rain_splats.clear();
			ground.particles.clear();
			ground.effects.clear();
			ground.pools.clear();
			let started = std::time::Instant::now();
			let n = quads_of(&ground).len();
			ground_build += started.elapsed();
			ground_quads += n;
		}
		quads += now.len();
		same_quads += now.iter().zip(&last_quads).filter(|(a, b)| a == b).count();
		same_prefix += now.iter().zip(&last_quads).take_while(|(a, b)| a == b).count();
		last_quads = now;
		if scene == last {
			still += 1;
			run += 1;
		} else {
			if run > 0 {
				runs.push(run);
			}
			run = 0;
			for what in differences(&last, &scene) {
				*changed.entry(what).or_default() += 1;
			}
		}
		last = scene;
	}
	if run > 0 {
		runs.push(run);
	}
	println!("{still} of {frames} frames show what the last did ({:.1}%)", 100.0 * still as f64 / frames as f64);
	runs.sort_unstable();
	if !runs.is_empty() {
		println!("still runs: {}, median {} frames, longest {}", runs.len(), runs[runs.len() / 2], runs[runs.len() - 1]);
	}
	println!(
		"quads a frame: {:.0}, the same as the last frame's at their place {:.1}%, before the first that differs {:.1}%; built in {:.3} ms",
		quads as f64 / frames as f64,
		100.0 * same_quads as f64 / quads as f64,
		100.0 * same_prefix as f64 / quads as f64,
		build.as_secs_f64() * 1000.0 / frames as f64
	);
	if ground_quads > 0 {
		println!("the ground alone: {:.0} quads, built in {:.3} ms", ground_quads as f64 / frames as f64, ground_build.as_secs_f64() * 1000.0 / frames as f64);
	}
	if !doublings.is_empty() {
		println!("steps (frames at each step of 1/60 s doubled k times): {doublings:?}");
	}
	println!("what changed, in how many frames:");
	for (what, n) in changed {
		println!("  {what}: {n}");
	}
}

/// What differs between two scenes, field by field (an entity's sprites by field).
fn differences(a: &TacticalScene, b: &TacticalScene) -> Vec<String> {
	let mut out = Vec::new();
	let mut field = |name: &str, same: bool| {
		if !same {
			out.push(name.to_string());
		}
	};
	field("size", (a.width, a.height) == (b.width, b.height));
	field("tiles", a.tiles == b.tiles);
	field("transitions", a.transitions == b.transitions);
	field("camera", a.camera == b.camera);
	field("light", a.light == b.light);
	field("clouds", a.clouds == b.clouds);
	field("hide_trees", a.hide_trees == b.hide_trees);
	field("particles", a.particles == b.particles);
	field("effects", a.effects == b.effects);
	field("pools", a.pools == b.pools);
	field("overlay_icons", a.overlay_icons == b.overlay_icons);
	field("path", a.path == b.path);
	field("blocked_tiles", a.blocked_tiles == b.blocked_tiles);
	field("rain", a.rain == b.rain);
	field("rain_splats", a.rain_splats == b.rain_splats);
	field("orientation", a.orientation == b.orientation);
	if a.tiles.len() == b.tiles.len() {
		let mut parts = std::collections::BTreeSet::new();
		for (x, y) in a.tiles.iter().zip(&b.tiles) {
			if x.details.len() != y.details.len() {
				parts.insert("tiles: detail count".to_string());
				continue;
			}
			for (d, e) in x.details.iter().zip(&y.details) {
				let brush = &d.brush;
				let mut part = |what: &str, same: bool| {
					if !same {
						parts.insert(format!("tiles: detail {what}"));
						parts.insert(format!("tiles: detail {brush}"));
					}
				};
				part("brush", d.brush == e.brush);
				part("color", d.color == e.color);
				part("offset", d.offset == e.offset);
				part("rotation", d.rotation == e.rotation);
				part("scale", d.scale == e.scale);
				part("saturation", d.saturation == e.saturation);
				part("frame", d.frame == e.frame);
			}
			let mut part = |what: &str, same: bool| {
				if !same {
					parts.insert(format!("tiles: {what}"));
				}
			};
			part("visible", x.visible == y.visible);
			part("discovered", x.discovered == y.discovered);
			part("highlight", x.highlight == y.highlight);
			part("unveils", x.unveils == y.unveils);
			part("color", x.color == y.color);
			part("level", x.level == y.level);
		}
		out.extend(parts);
	}
	if a.clouds.len() == b.clouds.len() && a.clouds != b.clouds {
		out.push(format!("clouds: {} of them, first {:?} -> {:?}", a.clouds.len(), a.clouds[0], b.clouds[0]).chars().take(60).collect());
	}
	if a.entities.len() != b.entities.len() {
		out.push("entities: count".into());
		return out;
	}
	let mut parts = std::collections::BTreeSet::new();
	for (x, y) in a.entities.iter().zip(&b.entities) {
		if x.pos != y.pos {
			parts.insert(format!("entities: pos (actor {})", x.is_actor));
			if std::env::var_os("SHOW_POS").is_some() {
				println!("{} {:?} -> {:?} flying {} tile {:?} sprites {:?}", x.id, x.pos, y.pos, x.flying, x.tile, x.sprites.iter().filter(|s| s.visible).map(|s| (&s.name, &s.brush)).collect::<Vec<_>>());
			}
		}
		if x.overlay != y.overlay {
			parts.insert("entities: overlay".to_string());
		}
		if x.sprites.len() != y.sprites.len() {
			parts.insert("entities: sprite count".to_string());
			continue;
		}
		for (s, t) in x.sprites.iter().zip(&y.sprites) {
			let name = &s.name;
			let mut part = |what: &str, same: bool| {
				if !same {
					parts.insert(format!("entities: sprite {name} {what}"));
				}
			};
			part("brush", s.brush == t.brush);
			part("color", s.color == t.color);
			part("scale", s.scale == t.scale);
			part("rotation", s.rotation == t.rotation);
			part("saturation", s.saturation == t.saturation);
			part("highlight", (s.highlight, s.highlight_factor) == (t.highlight, t.highlight_factor));
			part("visible", s.visible == t.visible);
			part("offset", s.offset == t.offset);
			part("frame", s.frame == t.frame);
		}
	}
	out.extend(parts);
	out
}

/// The battle `render_battle` shoots, up to the player's first turn, its camera on the
/// map's centre (and fog of war off unless `fog`).
fn start_battle(engine: &harness::GameEngine, fog: bool) {
	let frame = |engine: &harness::GameEngine| {
		engine.frame(1000.0 / 60.0).expect("frame");
		engine.pump_ui().expect("ui");
		engine.run_script("::__pilot();", "pilot").expect("pilot");
	};
	engine.run_script(include_str!("battle_setup.nut"), "setup").expect("setup");
	for _ in 0..120 {
		frame(engine);
	}
	engine.run_script("::__startBattle();", "battle").expect("battle");
	for _ in 0..6000 {
		frame(engine);
		engine.run_script("::__turn <- ::__playerTurn();", "turn").expect("turn");
		if squirrel::SqTable::root(engine.vm()).get::<bool>("__turn") == Some(true) {
			break;
		}
	}
	let camera = if fog { "::__battleCamera();" } else { "::__battleCamera(); ::__fogOff();" };
	engine.run_script(camera, "camera").expect("camera");
	for _ in 0..10 {
		frame(engine);
	}
}

fn take_value(args: &mut Vec<String>, flag: &str) -> Option<String> {
	let i = args.iter().position(|a| a == flag)?;
	args.remove(i);
	(i < args.len()).then(|| args.remove(i))
}

fn take_flag(args: &mut Vec<String>, flag: &str) -> bool {
	let found = args.iter().position(|a| a == flag);
	if let Some(i) = found {
		args.remove(i);
	}
	found.is_some()
}
