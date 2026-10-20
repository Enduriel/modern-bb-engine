//! A frame whose world scene key (`Engine::world_scene_key`) is the last one's shows the
//! same scene: `bb` leaves such a frame undrawn without building its scene, so every
//! change to what the scene shows must move the key. Checked every frame of a paused
//! map, a world running at 4x through a whole day (dusk and dawn), a town, a battle
//! fought by the AI and the map after it. Each change source also moves the key on its
//! own. Skips cleanly when the Steam install isn't there.
//!
//! `BB_SCENE_KEY_MODS=1` runs it with the install's mods.

use std::path::Path;

use bb_engine::harness::{self, GameEngine};
use squirrel_bb::scene::{WorldScene, WorldSceneKey};

const IN_BATTLE: &str = "((\"State\" in ::Tactical) && ::Tactical.State != null) ? 1 : 0";

fn open() -> Option<GameEngine> {
	let dir = Path::new(harness::STEAM_DATA_DIR);
	if !dir.exists() {
		eprintln!("skipping: {} not present", dir.display());
		return None;
	}
	let engine = if std::env::var_os("BB_SCENE_KEY_MODS").is_some() {
		harness::open_install_with_mods(dir)
	} else {
		harness::open_install(dir)
	}
	.expect("open install");
	harness::boot_menu(&engine).expect("boot the menu");
	harness::prepare_new_campaign(&engine, "rust-engine-test").expect("prepare the campaign");
	harness::start_new_campaign(&engine).expect("start the campaign");
	Some(engine)
}

/// Frames compared, and those whose key stayed (checked against the scene).
#[derive(Default)]
struct Check {
	last: Option<(WorldSceneKey, WorldScene)>,
	frames: usize,
	kept: usize,
	/// This phase's frames: those whose key stayed, and those whose key moved though
	/// the scene didn't (built for nothing), by what moved.
	phase: (usize, usize, std::collections::BTreeMap<&'static str, usize>),
}

impl Check {
	fn frame(&mut self, engine: &GameEngine, when: &str) {
		let key = engine.world_scene_key();
		let scene = drawn(engine.world_scene_where(|_| false));
		if let Some((last_key, last_scene)) = &self.last {
			if *last_key == key {
				self.kept += 1;
				self.phase.0 += 1;
				assert!(*last_scene == scene, "{when}: the key stayed but the scene changed: {}", difference(last_scene, &scene));
			} else if *last_scene == scene {
				let moved = if last_key.changes != key.changes {
					"changes"
				} else if last_key.virtual_time != key.virtual_time {
					"virtual time"
				} else if last_key.camera != key.camera {
					"camera"
				} else {
					"other"
				};
				*self.phase.2.entry(moved).or_default() += 1;
			}
		}
		self.frames += 1;
		self.phase.1 += 1;
		self.last = Some((key, scene));
	}

	/// Report the phase `name` just ended, and start the next.
	fn phase(&mut self, name: &str) -> usize {
		let (kept, frames, moved) = std::mem::take(&mut self.phase);
		eprintln!("{name}: {kept} of {frames} frames kept their key; moved with the scene the same: {moved:?}");
		kept
	}
}

/// What of `scene` is drawn: the entities shown, their visible sprites and labels (a
/// hidden sprite's fade runs on unseen).
fn drawn(mut scene: WorldScene) -> WorldScene {
	scene.entities.retain(|e| e.shown);
	for e in &mut scene.entities {
		e.sprites.retain(|s| s.visible && !s.brush.is_empty());
		e.labels.retain(|l| l.visible && !l.text.is_empty());
	}
	scene
}

/// Where two scenes first differ.
fn difference(a: &WorldScene, b: &WorldScene) -> String {
	if a.entities.len() != b.entities.len() {
		return format!("{} entities, then {}", a.entities.len(), b.entities.len());
	}
	for (x, y) in a.entities.iter().zip(&b.entities) {
		if x == y {
			continue;
		}
		if (x.id, x.pos, x.shown) != (y.id, y.pos, y.shown) {
			return format!("entity {}: {:?} {} then {:?} {}", x.id, x.pos, x.shown, y.pos, y.shown);
		}
		for (s, t) in x.sprites.iter().zip(&y.sprites) {
			if s != t {
				return format!("entity {} sprite: {s:?} then {t:?}", x.id);
			}
		}
		return format!("entity {}: {x:?} then {y:?}", x.id);
	}
	let parts = [
		("clouds", a.clouds != b.clouds),
		("birds", a.birds != b.birds),
		("waves", a.waves != b.waves),
		("particles", a.particles != b.particles),
		("footprints", a.footprints != b.footprints),
		("region names", a.region_texts != b.region_texts),
	];
	parts.iter().filter(|(_, differ)| *differ).map(|(name, _)| *name).collect::<Vec<_>>().join(", ")
}

fn frame(engine: &GameEngine) {
	harness::run_frame(engine, 1000.0 / 60.0).expect("frame");
}

#[test]
fn a_frame_with_the_last_key_shows_the_last_scene() {
	let Some(engine) = open() else { return };
	let mut check = Check::default();
	harness::install_autoplay(&engine).expect("autoplay");
	engine
		.run_script("::World.State.m.IsAutosaving = false; ::World.State.setPause(true);", "pause")
		.expect("pause");
	for i in 0..300 {
		frame(&engine);
		check.frame(&engine, &format!("paused, frame {i}"));
	}
	let paused_kept = check.phase("paused");
	assert!(paused_kept > 200, "a paused map keeps its key ({paused_kept} of 300 frames)");

	// A whole day at 4x: parties move, house lights come on at dusk and go at dawn,
	// entities come into sight and leave it, clouds and birds drift.
	harness::unpause_world(&engine).expect("unpause");
	engine.run_script("::World.setSpeedMult(4.0);", "4x").expect("4x");
	for i in 0..1800 {
		frame(&engine);
		check.frame(&engine, &format!("running at 4x, frame {i}"));
		if i % 60 == 0 {
			engine
				.run_script("if (::World.State.isPaused()) { ::World.State.setAutoPause(false); ::World.State.setPause(false); } ::World.setSpeedMult(4.0);", "run")
				.expect("run");
		}
	}
	check.phase("running at 4x");

	// Paused again, the camera moved about, then a town.
	engine.run_script("::World.State.setPause(true);", "pause").expect("pause");
	for i in 0..240 {
		if i % 40 == 0 {
			engine
				.run_script(&format!("local c = ::World.getCamera(); c.setPos(::createVec(c.getPos().X + {}.0, c.getPos().Y));", 50 * (i / 40)), "camera")
				.expect("camera");
		}
		frame(&engine);
		check.frame(&engine, &format!("paused again, frame {i}"));
	}
	check.phase("paused, the camera moved");
	engine
		.run_script(
			"local tile = ::World.State.getPlayer().getTile(); local nearest = null; local distance = 0; \
			 foreach (s in ::World.EntityManager.getSettlements()) { if (!s.isEnterable()) continue; \
			 local d = s.getTile().getDistanceTo(tile); if (nearest == null || d < distance) { nearest = s; distance = d; } } \
			 ::World.State.enterLocation(nearest);",
			"town",
		)
		.expect("town");
	for i in 0..300 {
		frame(&engine);
		check.frame(&engine, &format!("in a town, frame {i}"));
	}
	check.phase("town");
	engine.run_script("::World.State.getMenuStack().popAll(true);", "leave").expect("leave");

	// A battle fought by the AI, and the map after it.
	engine
		.run_script("::__autoplayFight = true; ::__autoplayDieAsPlayers = true;", "fight")
		.expect("fight");
	if let Err(e) = harness::engage_nearest_hostile(&engine) {
		// The mods' own screens hold the harness's engage up.
		assert!(std::env::var_os("BB_SCENE_KEY_MODS").is_some(), "engage: {e}");
		eprintln!("no battle with the mods: {e}");
		return;
	}
	let in_battle = || bb_engine::tactical_player::eval_i64(&engine, IN_BATTLE).ok().flatten() == Some(1);
	let (mut battle_frames, mut back) = (0, 0);
	for i in 0..20_000 {
		frame(&engine);
		check.frame(&engine, &format!("frame {i} after the engage"));
		if in_battle() {
			battle_frames += 1;
		} else if battle_frames > 0 {
			back += 1;
			if back == 300 {
				break;
			}
		}
	}
	assert!(battle_frames > 600, "fought a battle ({battle_frames} frames)");
	check.phase("battle and after");
	eprintln!("{} frames, {} kept their key", check.frames, check.kept);
}

/// Each source of change moves the key: an entity's sprite set, its label set, it
/// moved, a sprite added, the player's view moved, a footprint, a wave, a particle
/// effect, a region name, the camera, the light.
#[test]
fn each_change_moves_the_key() {
	let Some(engine) = open() else { return };
	harness::install_autoplay(&engine).expect("autoplay");
	engine
		.run_script("::World.State.m.IsAutosaving = false; ::World.State.setPause(true); ::__p <- ::World.State.getPlayer();", "pause")
		.expect("pause");
	for _ in 0..30 {
		frame(&engine);
	}
	let settle = || {
		// Until nothing moves on its own (fades, sight).
		let mut last = engine.world_scene_key();
		for _ in 0..600 {
			frame(&engine);
			let now = engine.world_scene_key();
			if now == last {
				return now;
			}
			last = now;
		}
		panic!("the paused map never settled");
	};
	let changes = [
		("a sprite's colour", "::__p.getSprite(\"body\").Color = ::createColor(\"#ff0000\");"),
		("a sprite hidden", "::__p.getSprite(\"body\").Visible = false;"),
		("a sprite's offset", "::__p.setSpriteOffset(\"body\", ::createVec(3, 4));"),
		("a sprite added", "::__p.addSprite(\"bb_probe\").setBrush(\"bust_base_militia\");"),
		("a label added", "::__l <- ::__p.addLabel(\"bb_probe\");"),
		("a label's text", "::__l.Text = \"probe text\";"),
		("moved", "::__p.setPos(::createVec(::__p.getPos().X + 10, ::__p.getPos().Y));"),
		("the player's view", "::World.setPlayerPos(::createVec(::__p.getPos().X + 30, ::__p.getPos().Y));"),
		("a footprint", "::World.spawnFootprint(::__p.getPos(), \"footprint_human_01\", 1.0, 5.0, 1.0, 0);"),
		("a wave", "::World.spawnWaveSprite(\"waves_01\", ::__p.getPos(), ::createVec(-15, -15), 1.25);"),
		("a region name", "::World.spawnRegionText(\"probe text\", 1.0, 0.0, ::__p.getPos());"),
		("the camera", "local c = ::World.getCamera(); c.setPos(::createVec(c.getPos().X + 5, c.getPos().Y));"),
	];
	for (what, code) in changes {
		let before = settle();
		engine.run_script(code, what).expect(what);
		let after = engine.world_scene_key();
		assert_ne!(before, after, "{what} moves the key");
	}
	settle();
	engine.run_script("::__p.fadeOut(300);", "fade").expect("fade");
	// A fade moves the key every frame until it's done.
	let mut last = engine.world_scene_key();
	for i in 0..10 {
		frame(&engine);
		let now = engine.world_scene_key();
		assert_ne!(last, now, "fading, frame {i}");
		last = now;
	}
}
