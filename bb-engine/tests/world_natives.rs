//! World natives built from the original's disassembly, on a new campaign:
//! `World.move`, `getPlayerPos`, `getAllTilesOfRegion`, `getAllRosters` and `roster.get`, the world tile's
//! `getTileBetweenThisAnd`/`getPosBetweenThisAnd`, and `presave`/`canLoad` around a save.
//! Then the Sqrat overloads probed on 2026-10-14: `getDistanceTo(x, y)`, `getEntityAtPos`,
//! and `spawnEntity(script)` at the tile under the mouse; and `IO.enumerateFiles` over
//! the real data folder.
//!
//! Skips if no Steam install. Slow in debug builds: it runs all of worldgen first.

use std::path::Path;

use bb_engine::{harness, tactical_player as player};
use squirrel_bb::MouseInput;

/// Run a check script that returns "" when everything holds, else what didn't.
fn check(engine: &harness::GameEngine, what: &str, script: &str) {
	let wrong = player::eval_string(engine, &format!("(function() {{ local wrong = []; {script}; return wrong.len() == 0 ? \"\" : wrong.reduce(@(a, b) a + \"; \" + b); }})()"))
		.unwrap_or_else(|e| panic!("{what}: {e}"))
		.unwrap_or_default();
	assert_eq!(wrong, "", "{what}");
}

#[test]
fn world_natives_from_the_disassembly() {
	let dir = Path::new(harness::STEAM_DATA_DIR);
	if !dir.exists() {
		eprintln!("skipping: {} not present", dir.display());
		return;
	}
	let engine = harness::open_install(dir).expect("open install");
	harness::boot_menu(&engine).expect("boot the menu");
	harness::prepare_new_campaign(&engine, "rust-engine-test").expect("prepare the campaign");
	harness::start_new_campaign(&engine).expect("startNewCampaign");

	check(
		&engine,
		"World.move",
		r#"
		local m = ::World.move(::createVec(10, 20), ::createVec(10, 20), 5.0);
		if (m.X != 10 || m.Y != 20) wrong.push("equal points: " + m.X + "," + m.Y);
		m = ::World.move(::createVec(0, 0), ::createVec(3, 4), 5.0);
		if (m.X != 3 || m.Y != 4) wrong.push("within reach: " + m.X + "," + m.Y);
		// f32 along the normalized direction, so not exactly 3,4.
		m = ::World.move(::createVec(0, 0), ::createVec(30, 40), 5.0);
		if (::fabs(m.X - 3) > 0.00001 || ::fabs(m.Y - 4) > 0.00001) wrong.push("a step of 5: " + m.X + "," + m.Y);
		"#,
	);
	check(
		&engine,
		"getPlayerPos",
		r#"
		::World.setPlayerPos(::createVec(100.1, 200.7));
		local p = ::World.getPlayerPos();
		// Stored as f32.
		if (::fabs(p.X - 100.1) > 0.0001 || ::fabs(p.Y - 200.7) > 0.0001) wrong.push("pos " + p.X + "," + p.Y);
		"#,
	);
	check(
		&engine,
		"getAllTilesOfRegion",
		r#"
		local region = ::World.State.getPlayer().getTile().Region;
		local tiles = ::World.getAllTilesOfRegion(region);
		if (tiles.len() == 0) wrong.push("no tiles in the player's region " + region);
		local prev = null, count = 0;
		foreach (t in tiles)
		{
			if (t.Region != region) wrong.push("tile of region " + t.Region);
			local key = t.SquareCoords.X * 100000 + t.SquareCoords.Y;
			if (prev != null && key <= prev) wrong.push("out of column order at " + t.SquareCoords.X + "," + t.SquareCoords.Y);
			prev = key;
		}
		local size = ::World.getMapSize();
		for (local x = 0; x < size.X; x++)
			for (local y = 0; y < size.Y; y++)
				if (::World.getTileSquare(x, y).Region == region) count++;
		if (count != tiles.len()) wrong.push(count + " tiles of the region, listed " + tiles.len());
		"#,
	);
	check(
		&engine,
		"getAllRosters",
		r#"
		// The created rosters only: the company's isn't among them.
		local rosters = ::World.getAllRosters();
		if (rosters.len() == 0) wrong.push("no rosters");
		local company = ::World.getPlayerRoster().getAll();
		foreach (r in rosters)
			foreach (u in r.getAll())
				foreach (b in company)
					if (u == b) wrong.push("the company's roster is listed");
		// roster.get(i): the i-th member, null past the end.
		local roster = ::World.getPlayerRoster();
		if (roster.get(0) != company[0]) wrong.push("get(0)");
		local past = roster.get(company.len()), neg = roster.get(-1);
		if (typeof past != "table" || past.len() != 0) wrong.push("get past the end: " + typeof past);
		if (typeof neg != "table" || neg.len() != 0) wrong.push("get(-1): " + typeof neg);
		"#,
	);
	check(
		&engine,
		"WorldTile between",
		r#"
		local t = ::World.getTileSquare(20, 20);
		// A tile and itself: NaN, which picks tile 0,0 (as the original).
		local self = t.getTileBetweenThisAnd(t);
		if (self.SquareCoords.X != 0 || self.SquareCoords.Y != 0) wrong.push("self: " + self.SquareCoords.X + "," + self.SquareCoords.Y);
		for (local d = 0; d < 6; d++)
		{
			local mid = t.getNextTile(d), far = mid.getNextTile(d);
			if (!t.getTileBetweenThisAnd(far).isSameTileAs(mid)) wrong.push("dir " + d);
			local p = t.getPosBetweenThisAnd(far);
			if (p.X != mid.Pos.X || p.Y != mid.Pos.Y) wrong.push("pos " + d);
		}
		"#,
	);

	// presave: the next save waits for the world to be drawn.
	check(
		&engine,
		"presave",
		r#"
		::World.presave();
		::World.save("presave_test", null);
		if (::World.canLoad("presave_test")) wrong.push("saved before the world was drawn");
		"#,
	);
	harness::run_frame(&engine, 1000.0 / 60.0).expect("frame");
	check(
		&engine,
		"save after presave",
		r#"
		::World.save("presave_test", null);
		if (!::World.canLoad("presave_test")) wrong.push("not saved after the world was drawn");
		if (::World.canLoad("no_such_save")) wrong.push("canLoad of a missing save");
		"#,
	);

	check(
		&engine,
		"getDistanceTo(x, y)",
		r#"
		// Probed: the (x, y) form takes axial Coords, and floats truncate.
		local t0 = ::World.getTileSquare(20, 20);
		if (t0.Coords.X != 20 || t0.Coords.Y != 10) wrong.push("t0 coords " + t0.Coords.X + "," + t0.Coords.Y);
		foreach (c in [[23, 20, 3], [20, 23, 3], [22, 21, 2], [25, 24, 7], [17, 22, 4], [27, 15, 8]])
		{
			local t = ::World.getTileSquare(c[0], c[1]);
			if (t0.getDistanceTo(t) != c[2]) wrong.push("tile form to " + c[0] + "," + c[1]);
			if (t0.getDistanceTo(t.Coords.X, t.Coords.Y) != c[2]) wrong.push("xy form to " + c[0] + "," + c[1]);
		}
		if (t0.getDistanceTo(0, 0) != 30 || t0.getDistanceTo(-5, 3) != 32 || t0.getDistanceTo(1000, 1000) != 1970) wrong.push("raw coords");
		if (t0.getDistanceTo(20.7, 10.2) != 0) wrong.push("floats don't truncate");
		try { t0.getDistanceTo(); wrong.push("no args accepted"); }
		catch (e) { if (e != "No overload matching this argument list found") wrong.push("no args: " + e); }
		"#,
	);
	check(
		&engine,
		"getEntityAtPos",
		r#"
		local p = ::World.State.getPlayer().getPos();
		if (::World.getEntityAtPos(p) != null || ::World.getEntityAtPos(p.X, p.Y) != null) wrong.push("found an entity");
		"#,
	);

	// spawnEntity(script) spawns at the tile under the mouse, once it has been over the map.
	check(
		&engine,
		"spawnEntity before the mouse moved",
		r#"try { ::World.spawnEntity("scripts/entity/world/party"); wrong.push("spawned"); } catch (e) {}"#,
	);
	engine.root_state().mouse_input(engine.vm(), MouseInput::moved(700, 300)).expect("mouse move");
	check(
		&engine,
		"spawnEntity at the hovered tile",
		r#"
		local hovered = ::World.screenToTile(700, 300);
		local e = ::World.spawnEntity("scripts/entity/world/party");
		local c = e.getTile().Coords;
		if (c.X != hovered.X || c.Y != hovered.Y) wrong.push("at " + c.X + "," + c.Y + ", not " + hovered.X + "," + hovered.Y);
		e.die();
		"#,
	);
	check(
		&engine,
		"IO.enumerateFiles over the data folder",
		r#"
		local files = ::IO.enumerateFiles("data/");
		if (files == null || files.find("data/data_001") == null) wrong.push("data/data_001 not listed");
		if (::IO.enumerateFiles("./data/") != null) wrong.push("./data/ listed");
		"#,
	);
}
