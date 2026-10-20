//! A unit spawned in the middle of a battle that rises from the ground
//! (`actor.riseFromGround`, the necromancer's raised dead, Reforged's barrowkin) is
//! drawn on its tile. The script moves it with `setPos(0, -200 → 0)` from its
//! `onRender`, and in the original that position is relative to the unit's tile
//! (`TacticalEntity::setPos`, `0x542070`, writes the scene node's local position;
//! `getPos`, `0x4c2c30`, adds the tile's world position while the unit stands on one).
//! Taken as a world position it put the unit at the map's corner, where nobody saw it.
//! Skips cleanly without the Steam install.

use std::path::Path;

use bb_engine::harness;
use bb_render::tactical::tactical_quads;
use bb_render::world::WorldView;

fn frames(engine: &harness::GameEngine, n: usize) {
	for frame in 0..n {
		harness::run_frame(engine, 1000.0 / 60.0).unwrap_or_else(|e| panic!("frame {frame}: {e}"));
	}
}

/// Where the risen unit is drawn (its scene position) and where its tile is.
fn rising(engine: &harness::GameEngine, id: i64) -> ((f64, f64), (f64, f64)) {
	let scene = engine.tactical_scene();
	let unit = scene.entities.iter().find(|e| e.id == id).expect("the risen unit is in the scene");
	let tile = &scene.tiles[unit.tile.expect("on a tile")];
	(unit.pos, (tile.pos.0, tile.pos.1 + 40.0 * tile.level as f64))
}

#[test]
fn a_unit_spawned_mid_battle_rises_on_its_tile() {
	let dir = Path::new(harness::STEAM_DATA_DIR);
	if !dir.exists() {
		eprintln!("skipping: {} not present", dir.display());
		return;
	}
	let engine = harness::open_install(dir).expect("open install");
	harness::boot_menu(&engine).expect("boot the menu");
	harness::prepare_new_campaign(&engine, "rust-engine-test").expect("prepare the campaign");
	harness::start_new_campaign(&engine).expect("startNewCampaign");
	harness::unpause_world(&engine).expect("unpause");
	frames(&engine, 60);
	harness::engage_nearest_hostile(&engine).expect("engage");
	// Until the battle is on and the player's units stand on the map.
	for frame in 0..3_000 {
		if engine.tactical_scene().entities.iter().any(|e| e.is_actor && e.overlay.is_some()) {
			break;
		}
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap_or_else(|e| panic!("frame {frame}: {e}"));
	}
	// A zombie rises next to a brother, where the player sees it rise (an empty tile in
	// sight whose southern neighbour is discovered and not lower: `riseFromGround`'s
	// rising branch rather than its fade-in).
	engine
		.run_script(
			r#"
			::__risen <- null;
			local S = ::Const.Direction.S;
			foreach (bro in ::Tactical.Entities.getInstancesOfFaction(::Const.Faction.Player))
			{
				local at = bro.getTile();
				for (local d = 0; d < 6 && ::__risen == null; d++)
				{
					if (!at.hasNextTile(d)) continue;
					local t = at.getNextTile(d);
					if (!t.IsEmpty || !t.IsVisibleForPlayer || !t.hasNextTile(S)) continue;
					local s = t.getNextTile(S);
					if (t.Level > s.Level || !s.IsDiscovered) continue;
					local e = ::Tactical.spawnEntity("scripts/entity/tactical/enemies/zombie", t.Coords.X, t.Coords.Y);
					e.setFaction(::Const.Faction.Undead);
					e.riseFromGround();
					if (!e.m.IsRaising) throw "the zombie didn't take the rising branch";
					::__risen = e.getID();
				}
				if (::__risen != null) break;
			}
			if (::__risen == null) throw "no tile to rise on next to a brother";
			"#,
			"rise",
		)
		.expect("a zombie rises");
	let id = squirrel::SqTable::root(engine.vm()).get::<i64>("__risen").expect("its id");

	// Half way up (ResurrectAnimationTime is 1 s): on its tile's column, below it.
	frames(&engine, 30);
	let (pos, tile) = rising(&engine, id);
	assert_eq!(pos.0, tile.0, "rises in its tile's column: at {pos:?}, tile at {tile:?}");
	assert!(pos.1 < tile.1 && pos.1 > tile.1 - 200.0, "still coming up: at {pos:?}, tile at {tile:?}");

	// Risen: on its tile, and the scripts read the same positions as the original's.
	frames(&engine, 60);
	let (pos, tile) = rising(&engine, id);
	assert_eq!(pos, tile, "stands on its tile");
	// And the renderer draws its sprites there: the quads the unit adds to a view of
	// its tile sit on and over the tile.
	let scene = engine.tactical_scene();
	let mut without = scene.clone();
	without.entities.retain(|e| e.id != id);
	let view = WorldView {
		center: [tile.0 as f32, tile.1 as f32],
		zoom: 1.0,
		width: 800,
		height: 800,
	};
	let (all, rest) = engine.sprites().with(|registry| {
		(tactical_quads(&scene, registry, &view).quads, tactical_quads(&without, registry, &view).quads)
	});
	let unit: Vec<[f32; 2]> = all
		.iter()
		.filter(|q| !rest.iter().any(|r| r.corners == q.corners && r.uv == q.uv))
		.map(|q| {
			let c = q.corners;
			[c.iter().map(|p| p[0]).sum::<f32>() / 4.0, c.iter().map(|p| p[1]).sum::<f32>() / 4.0]
		})
		.collect();
	assert!(unit.len() >= 3, "the unit's sprites are drawn ({} quads)", unit.len());
	let (x, y) = (tile.0 as f32, tile.1 as f32);
	for c in &unit {
		assert!((c[0] - x).abs() < 100.0 && c[1] > y - 60.0 && c[1] < y + 200.0, "a sprite at {c:?}, its tile at {tile:?}");
	}
	engine
		.run_script(
			&format!(
				"local e = ::Tactical.getEntityByID({id}); \
				 if (e.m.IsRaising) throw \"still rising\"; \
				 local p = e.getPos(), t = e.getTile().Pos, r = e.getRelativePos(); \
				 if (p.X != t.X || p.Y != t.Y) throw \"getPos \" + p.X + \",\" + p.Y + \" is not the tile's \" + t.X + \",\" + t.Y; \
				 if (r.X != 0 || r.Y != 0) throw \"getRelativePos \" + r.X + \",\" + r.Y;"
			),
			"positions",
		)
		.expect("positions");
}
