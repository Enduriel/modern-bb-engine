//! `Tactical.worldToTile` against the original: 22,164 world points on a 32x32 battle
//! map, first with its generated heights (levels 0 and 1), then with tiles raised to
//! levels 1 to 3, the camera at level 1 (`testdata/world_to_tile.txt`, captured from the
//! original game 2026-10-13). The picking masks come from the game data, so this
//! also covers reading them through the archives.
//!
//! Fixture lines: `L x levels…` / `M x levels…` give each column's tile levels (the
//! generated map, then the raised one); `W`/`V` lines (sampled on the map from `L`/`M`)
//! give `x0 y step` and the original's answers for `x0 + step·i`.
//!
//! Skips if no Steam install.

use std::path::Path;

use bb_engine::harness;
use squirrel::SqTable;

const FIXTURE: &str = include_str!("testdata/world_to_tile.txt");

/// A script setting every tile's level from the fixture's `tag` lines.
fn set_levels(tag: &str) -> String {
	let mut script = String::new();
	for line in FIXTURE.lines().filter(|l| l.starts_with(tag)) {
		let mut f = line.split(' ').skip(1);
		let x = f.next().unwrap();
		for (y, level) in f.next().unwrap().split(',').filter(|v| !v.is_empty()).enumerate() {
			script += &format!("::Tactical.getTileSquare({x}, {y}).Level = {level};\n");
		}
	}
	script
}

#[test]
fn world_to_tile_matches_the_original() {
	let dir = Path::new(harness::STEAM_DATA_DIR);
	if !dir.exists() {
		eprintln!("skipping: {} not present", dir.display());
		return;
	}
	let engine = harness::open_install(dir).expect("open install");
	engine
		.run_script("::Tactical.resizeScene(32, 32); ::Tactical.getCamera().Level = 1;", "map")
		.expect("map");
	let mut points = 0;
	for (levels, samples) in [("L ", "W "), ("M ", "V ")] {
		engine.run_script(&set_levels(levels), "levels").expect("levels");
		for line in FIXTURE.lines().filter(|l| l.starts_with(samples)) {
			let f: Vec<&str> = line.split(' ').filter(|v| !v.is_empty()).collect();
			let (x0, y, step) = (f[1], f[2], f[3]);
			let want = &f[4..];
			engine
				.run_script(
					&format!(
						"local s = []; for (local i = 0; i < {n}; i++) {{ local c = ::Tactical.worldToTile(::createVec({x0} + {step} * i, {y})); s.push(c.X + \",\" + c.Y); }} ::__picked <- s;",
						n = want.len()
					),
					"pick",
				)
				.expect("pick");
			let picked = SqTable::root(engine.vm()).get::<squirrel::SqArray>("__picked").expect("picked");
			for (i, w) in want.iter().enumerate() {
				let got = picked.get::<String>(i).expect("coords");
				assert_eq!(&got, w, "{samples}x = {x0} + {step}·{i}, y = {y}");
			}
			points += want.len();
		}
	}
	assert_eq!(points, 22_164);
}

/// `tile.getTileBetweenThisAnd(other)` picks the tile at the midpoint of the two (the
/// original's, disassembled): two steps in any direction on flat ground give the tile
/// in between, and a tile and itself give that tile.
#[test]
fn the_tile_between_two_tiles_is_picked_at_their_midpoint() {
	let dir = Path::new(harness::STEAM_DATA_DIR);
	if !dir.exists() {
		eprintln!("skipping: {} not present", dir.display());
		return;
	}
	let engine = harness::open_install(dir).expect("open install");
	engine
		.run_script(
			r#"
			::Tactical.resizeScene(32, 32);
			::Tactical.getCamera().Level = 0;
			local t = ::Tactical.getTileSquare(12, 12);
			local wrong = [];
			if (!t.getTileBetweenThisAnd(t).isSameTileAs(t)) wrong.push("self");
			for (local d = 0; d < 6; d++)
			{
				local mid = t.getNextTile(d), far = mid.getNextTile(d);
				if (!t.getTileBetweenThisAnd(far).isSameTileAs(mid)) wrong.push("dir " + d);
				local p = t.getPosBetweenThisAnd(far);
				if (p.X != mid.Pos.X || p.Y != mid.Pos.Y) wrong.push("pos " + d);
			}
			::__wrong <- wrong.len() == 0 ? "" : wrong.reduce(@(a, b) a + ", " + b);
			"#,
			"between",
		)
		.expect("between");
	let wrong = SqTable::root(engine.vm()).get::<String>("__wrong").expect("result");
	assert_eq!(wrong, "", "wrong tiles between");
}
