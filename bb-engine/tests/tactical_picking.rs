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

/// The probe's tile pairs: from each sampled tile to these square-coord offsets.
const BETWEEN_DELTAS: [(i64, i64); 14] =
	[(1, 0), (0, 1), (2, 1), (1, 2), (3, 0), (0, 3), (2, -2), (-2, 3), (4, 1), (-3, -1), (5, 2), (1, -4), (6, 0), (-1, 5)];

/// `getTileBetweenThisAnd` against the original, on a battle map it generated
/// (`testdata/tile_between.txt`, captured from the original game 2026-10-14): its
/// levels (`L x levels…`), the camera (`C level flipped`), and the tile between a tile
/// and each of [`BETWEEN_DELTAS`] (`B x y answers…`, `-` off the map). The capture also
/// holds each unit's `isOccludedByTerrain` (`O x,y bool`): all false on this nearly
/// flat map, as the rule gives.
#[test]
fn tile_between_matches_the_original() {
	const DATA: &str = include_str!("testdata/tile_between.txt");
	let dir = Path::new(harness::STEAM_DATA_DIR);
	if !dir.exists() {
		eprintln!("skipping: {} not present", dir.display());
		return;
	}
	let engine = harness::open_install(dir).expect("open install");
	let camera = DATA.lines().find_map(|l| l.strip_prefix("C ")).expect("camera line");
	let (level, flipped) = camera.split_once(' ').expect("level flipped");
	engine.set_battle_camera_flipped(flipped == "true");
	let mut script = format!("::Tactical.resizeScene(32, 32); ::Tactical.getCamera().Level = {level};\n");
	for line in DATA.lines().filter(|l| l.starts_with("L ")) {
		let mut f = line.split(' ').skip(1);
		let x = f.next().unwrap();
		for (y, lv) in f.next().unwrap().split(',').filter(|v| !v.is_empty()).enumerate() {
			script += &format!("::Tactical.getTileSquare({x}, {y}).Level = {lv};\n");
		}
	}
	engine.run_script(&script, "levels").expect("levels");
	let mut checked = 0;
	let mut wrong = Vec::new();
	for line in DATA.lines().filter(|l| l.starts_with("B ")) {
		let f: Vec<&str> = line.split(' ').collect();
		let (sx, sy): (i64, i64) = (f[1].parse().unwrap(), f[2].parse().unwrap());
		for (&(dx, dy), want) in BETWEEN_DELTAS.iter().zip(&f[3..]) {
			if *want == "-" {
				continue;
			}
			let got = bb_engine::tactical_player::eval_string(
				&engine,
				&format!(
					"(function() {{ local m = ::Tactical.getTileSquare({sx}, {sy}).getTileBetweenThisAnd(::Tactical.getTileSquare({}, {})); return m.SquareCoords.X + \",\" + m.SquareCoords.Y; }})()",
					sx + dx,
					sy + dy
				),
			)
			.unwrap()
			.unwrap();
			checked += 1;
			if got != *want {
				wrong.push(format!("({sx},{sy})→({},{}): {got}, original {want}", sx + dx, sy + dy));
			}
		}
	}
	assert!(checked > 300, "only {checked} pairs");
	assert!(wrong.is_empty(), "{} of {checked} differ: {wrong:?}", wrong.len());
}
