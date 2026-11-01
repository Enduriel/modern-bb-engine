//! The two vector types, as in the original (probed 2026-11-01). Tile coordinates are
//! `vec2i` instances: a tile's `Coords`/`SquareCoords`, `getMapSize()` and
//! `worldToTile()` all hand one back, a fresh copy each time, with integer `X`/`Y` that
//! truncate what is written to them, and none of a table's behaviour (no new slots, no
//! `in`, no `foreach`). Positions are `vec2f`s. A native takes only its own type: the
//! original reads any instance's payload unchecked (and crashes on a table), vanilla
//! never mixes them, and here the other type is a script error that says what to pass.

use std::collections::HashMap;

use squirrel_bb::{Engine, LoadedScript, ScriptSource};

struct NoScripts;

impl ScriptSource for NoScripts {
	fn load(&self, _path: &str) -> Option<LoadedScript> {
		None
	}
	fn list_prefix(&self, _prefix: &str) -> Option<Vec<String>> {
		None
	}
}

fn engine() -> Engine<NoScripts> {
	let engine = Engine::new(NoScripts);
	engine.run_script("World.resizeScene(8, 8);", "setup").expect("a world map");
	engine
}

#[test]
fn coordinates_are_vec2i() {
	let checks: HashMap<&str, &str> = HashMap::from([
		("Coords", "World.getTile(2, 3).Coords"),
		("SquareCoords", "World.getTileSquare(2, 4).SquareCoords"),
		("getMapSize", "World.getMapSize()"),
		("worldToTile", "World.worldToTile(World.getTile(2, 3).Pos)"),
	]);
	let engine = engine();
	for (what, expr) in checks {
		engine
			.run_script(
				&format!(
					r#"
					local c = {expr};
					assert(typeof c == "instance");
					assert(c instanceof ::vec2i);
					assert(typeof c.X == "integer" && typeof c.Y == "integer");
					"#
				),
				what,
			)
			.unwrap_or_else(|e| panic!("{what}: {e}"));
	}
	engine
		.run_script(
			r#"
			local c = World.getTile(2, 3).Coords;
			assert(c.X == 2 && c.Y == 3);
			local m = World.getMapSize();
			assert(m.X == 8 && m.Y == 8);
			"#,
			"values",
		)
		.expect("the values");
}

#[test]
fn coordinates_behave_like_the_originals() {
	let engine = engine();
	engine
		.run_script(
			r#"
			local t = World.getTile(2, 3);
			assert(t.Coords != t.Coords);       // a fresh copy each time
			local c = t.Coords;
			c.X = 5;
			assert(c.X == 5 && t.Coords.X == 2); // the copy changes, the tile doesn't
			c.X = 1.7;
			assert(c.X == 1);                    // truncated
			c.X = "a";
			assert(c.X == 0);                    // a non-number is 0 (the original: garbage)
			assert(!("X" in c));
			local slot = false;
			try { c.Z <- 1; } catch (e) { slot = true; }
			assert(slot);
			local iterated = false;
			try { foreach (k, v in c) {} } catch (e) { iterated = true; }
			assert(iterated);
			"#,
			"semantics",
		)
		.expect("vec2i semantics");
}

#[test]
fn natives_take_only_their_vector_type() {
	let engine = engine();
	// Each right type is taken.
	engine
		.run_script(
			r#"
			local t = World.getTile(2, 3);
			World.uncoverFogOfWar(t.Pos, 100.0);
			World.getAllEntitiesAtPos(t.Pos, 100.0);
			assert(World.getTile(World.worldToTile(t.Pos)).Coords.X == 2);
			assert(World.tileToWorld(t.Coords).X == t.Pos.X);
			"#,
			"right",
		)
		.expect("each native takes its own type");
	let wrong = [
		("World.uncoverFogOfWar(World.getTile(2, 3).Coords, 100.0)", "uncoverFogOfWar: expected a vector, a vec2f like createVec makes (got a vec2i"),
		("World.getAllEntitiesAtPos(World.getTileSquare(2, 3), 100.0)", "getAllEntitiesAtPos: expected a vector, a vec2f"),
		("World.worldToTile({ X = 10, Y = 20 })", "worldToTile: expected a vector, a vec2f"),
		("World.getTile(createVec(2, 3))", "getTile: expected tile coordinates, a vec2i like tile.Coords (got a vec2f"),
		("World.tileToWorld({ X = 2, Y = 3 })", "tileToWorld: expected tile coordinates, a vec2i"),
	];
	for (code, says) in wrong {
		let err = engine.run_script(code, "wrong").expect_err(code);
		assert!(err.contains(says), "{code}: {err}");
	}
}
