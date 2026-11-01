//! World natives built from the original's disassembly, on a new campaign:
//! `World.move`, `getPlayerPos`, `getAllTilesOfRegion`, `getAllRosters` and `roster.get`, the world tile's
//! `getTileBetweenThisAnd`/`getPosBetweenThisAnd`, and `presave`/`canLoad` around a save.
//! `getAllEntitiesAtPos` given a world tile (an error here; the origin in the original).
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
		bb_engine::harness::skip(format!("{} not present", dir.display()));
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
	// A world tile as the position (Reforged 0.9.3's contract details): the original reads
	// its pointer bits as the centre, the map's origin in practice (probed 2026-10-21). We
	// take only a vec2f there (an intentional divergence): a script error saying so.
	check(
		&engine,
		"getAllEntitiesAtPos(tile)",
		r#"
		local tile = ::World.State.getPlayer().getTile();
		local err = null;
		try { ::World.getAllEntitiesAtPos(tile, 400); } catch (e) { err = e; }
		if (err == null || err.find("expected a vector, a vec2f") == null) wrong.push("a tile is taken: " + err);
		if (::World.getAllEntitiesAtPos(tile.Pos, 400).len() == 0) wrong.push("nothing around the player's own position");
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
		::World.save("bb_probe_presave_test", null);
		if (::World.canLoad("bb_probe_presave_test")) wrong.push("saved before the world was drawn");
		"#,
	);
	harness::run_frame(&engine, 1000.0 / 60.0).expect("frame");
	check(
		&engine,
		"save after presave",
		r#"
		::World.save("bb_probe_presave_test", null);
		if (!::World.canLoad("bb_probe_presave_test")) wrong.push("not saved after the world was drawn");
		if (::World.canLoad("bb_probe_no_such_save")) wrong.push("canLoad of a missing save");
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
	// Contracts hold their Home (and much else) as a WeakTableRef: the original's natives
	// resolve it to the entity (`_get` forwards), so a defend contract's
	// `party.getDistanceTo(m.Home) <= 700.0` measures to the town, not 0.
	check(
		&engine,
		"entity natives given a WeakTableRef",
		r#"
		local player = ::World.State.getPlayer();
		local far = null;
		foreach (s in ::World.EntityManager.getSettlements())
			if (far == null || player.getDistanceTo(s) > player.getDistanceTo(far)) far = s;
		local ref = ::WeakTableRef(far);
		local d = player.getDistanceTo(far);
		if (d < 1000) wrong.push("farthest settlement only " + d + " away");
		if (player.getDistanceTo(ref) != d) wrong.push("getDistanceTo: " + player.getDistanceTo(ref) + " through the ref, " + d + " direct");
		if (player.getDirectionTo(ref) != player.getDirectionTo(far)) wrong.push("getDirectionTo");
		if (player.isVisibleToEntity(ref, 1.0) != player.isVisibleToEntity(far, 1.0)) wrong.push("isVisibleToEntity");
		if (::WeakTableRef(player).getDistanceTo(far) != d) wrong.push("called on a ref");
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
	check(
		&engine,
		"createRoster",
		r#"
		// It returns the roster (made if new); a float id reads as its integer.
		local r = ::World.createRoster(424242);
		if (r == null || r.getSize() != 0) wrong.push("createRoster didn't return the new roster");
		r.create("scripts/entity/tactical/player");
		if (::World.getRoster(424242.7).getSize() != 1) wrong.push("a float id isn't read as its integer");
		::World.deleteRoster(424242);
		if (::World.getRoster(424242).getSize() != 0) wrong.push("deleteRoster left the roster");
		// An id that isn't a number is an error (the original reads garbage, so every
		// such id names one shared roster: an intentional divergence).
		foreach (call in [
			["createRoster", @() ::World.createRoster("mod_clever_recruiter"), "string"],
			["getRoster", @() ::World.getRoster(null), "null"],
			["deleteRoster", @() ::World.deleteRoster({}), "table"],
		]) {
			local err = null;
			try { call[1](); } catch (e) { err = e; }
			local expected = call[0] + ": the roster id must be an integer (got " + call[2] + ")";
			if (err == null || err.find(expected) == null) wrong.push(call[0] + " took a " + call[2] + ": " + err);
		}
		"#,
	);
}
