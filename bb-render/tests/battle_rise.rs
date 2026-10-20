//! A unit spawned in the middle of a battle that rises from the ground
//! (`actor.riseFromGround`, the necromancer's raised dead, Reforged's barrowkin) is
//! drawn on its tile. The script moves it with `setPos(0, -200 → 0)` from its
//! `onRender`, and in the original that position is relative to the unit's tile
//! (`TacticalEntity::setPos`, `0x542070`, writes the scene node's local position;
//! `getPos`, `0x4c2c30`, adds the tile's world position while the unit stands on one).
//! Taken as a world position it put the unit at the map's corner, where nobody saw it.
//! While it rises, the tiles in front of it hide what is still below the ground.
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

/// The quads unit `id` adds to a view of `tile` (what the renderer draws of it and its
/// bars), each with its place in the draw order, and every quad of that view.
fn unit_quads(engine: &harness::GameEngine, id: i64, tile: (f64, f64)) -> (Vec<(usize, [[f32; 2]; 4])>, Vec<[[f32; 2]; 4]>) {
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
	let unit = all
		.iter()
		.enumerate()
		.filter(|(_, q)| !rest.iter().any(|r| r.corners == q.corners && r.uv == q.uv))
		.map(|(i, q)| (i, q.corners))
		.collect();
	(unit, all.iter().map(|q| q.corners).collect())
}

/// Whether the quad with these corners covers `p`.
fn covers(corners: &[[f32; 2]; 4], p: [f32; 2]) -> bool {
	let (xs, ys) = (corners.map(|c| c[0]), corners.map(|c| c[1]));
	let (min, max) = (|v: [f32; 4]| v.iter().copied().fold(f32::INFINITY, f32::min), |v: [f32; 4]| v.iter().copied().fold(f32::NEG_INFINITY, f32::max));
	p[0] >= min(xs) && p[0] <= max(xs) && p[1] >= min(ys) && p[1] <= max(ys)
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
	// Its upper part shows over its tile; what is below the tile in front of it is drawn
	// over by that tile, which comes after it in the draw order (the original's tile
	// pass draws a tile's units right after the tile, and doesn't depth test).
	let (quads, all) = unit_quads(&engine, id, tile);
	let (x, y) = (tile.0 as f32, tile.1 as f32);
	assert!(quads.iter().any(|(_, q)| q.iter().any(|p| p[1] > y)), "its upper part shows while it rises");
	let sunk = [x, y - 60.0];
	let last = quads.iter().filter(|(_, q)| covers(q, sunk)).map(|&(i, _)| i).max().expect("its sunk part is drawn");
	assert!(
		all[last + 1..].iter().any(|q| covers(q, sunk)),
		"the tile in front draws over its sunk part at {sunk:?}, its tile at {tile:?}"
	);

	// Risen: on its tile, and the scripts read the same positions as the original's.
	frames(&engine, 60);
	let (pos, tile) = rising(&engine, id);
	assert_eq!(pos, tile, "stands on its tile");
	// And the renderer draws its sprites there, on and over the tile.
	let centres: Vec<[f32; 2]> = unit_quads(&engine, id, tile)
		.0
		.iter()
		.map(|(_, c)| [c.iter().map(|p| p[0]).sum::<f32>() / 4.0, c.iter().map(|p| p[1]).sum::<f32>() / 4.0])
		.collect();
	assert!(centres.len() >= 3, "the unit's sprites are drawn ({} quads)", centres.len());
	for c in &centres {
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

	// A script raises its tile a level (the frame scenes' `orient` does a brother's):
	// the unit rises with it, drawn and read where the tile is now, as in the
	// original's screenshot (its bars and all 40 px up), since its position is kept
	// relative to its tile.
	engine
		.run_script(&format!("::Tactical.getEntityByID({id}).getTile().Level += 1;"), "raise")
		.expect("raise its tile");
	let (pos, raised) = rising(&engine, id);
	assert_eq!(raised, (tile.0, tile.1 + 40.0), "the tile is a level up");
	assert_eq!(pos, raised, "the unit stands on its raised tile");
	engine
		.run_script(
			&format!(
				"local e = ::Tactical.getEntityByID({id}); \
				 local p = e.getPos(), t = e.getTile().Pos, r = e.getRelativePos(); \
				 if (p.X != t.X || p.Y != t.Y) throw \"getPos \" + p.X + \",\" + p.Y + \" is not the raised tile's \" + t.X + \",\" + t.Y; \
				 if (r.X != 0 || r.Y != 0) throw \"getRelativePos \" + r.X + \",\" + r.Y;"
			),
			"raised positions",
		)
		.expect("raised positions");
}
