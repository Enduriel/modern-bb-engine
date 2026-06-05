//! The World entity host. `World.spawnLocation` instantiates a real-shaped
//! location class (inherit chain + create/onInit/onAfterInit), injects the engine
//! primitives (getID/getTile/getPos/addSprite/addLabel), registers it in the (script)
//! EntityManager, and binds its tile. Self-contained in-memory source (no game data).

use std::collections::HashMap;

use squirrel_bb::{Engine, LoadedScript, ScriptSource};

struct MemSource(HashMap<String, String>);

impl ScriptSource for MemSource {
	fn load(&self, path: &str) -> Option<LoadedScript> {
		self.0
			.get(path)
			.map(|s| LoadedScript::Source(s.clone().into_bytes()))
	}
	fn list_prefix(&self, _prefix: &str) -> Option<Vec<String>> {
		None
	}
}

fn sources() -> MemSource {
	let mut m = HashMap::new();
	// A minimal real-shaped EntityManager (plain table, created via `new`, like vanilla).
	m.insert(
		"scripts/entity/world/entity_manager".into(),
		r#"
		this.entity_manager <- {
			m = { Locations = [], Settlements = [] },
			function addLocation( _l )    { this.m.Locations.push(_l); }
			function getLocations()       { return this.m.Locations; }
			function addSettlement( _s )  { this.m.Settlements.push(_s); }
			function getSettlements()     { return this.m.Settlements; }
		};
		"#
		.into(),
	);
	// Base world entity: onInit/onAfterInit exercise the injected natives the real
	// chain uses (Math.rand, addSprite).
	m.insert(
		"scripts/entity/world/world_entity".into(),
		r#"
		this.world_entity <- {
			m = { Name = "Test Site", CombatSeed = 0 },
			function getName() { return this.m.Name; }
			function onInit() {
				if (this.m.CombatSeed == 0) this.m.CombatSeed = this.Math.rand();
			}
			function onAfterInit() {
				local body = this.addSprite("body");
				body.Visible = false;
			}
		};
		"#
		.into(),
	);
	// Location: the real onInit shape — super-call, EntityManager registration, tile
	// occupancy, label.
	m.insert(
		"scripts/entity/world/location".into(),
		r#"
		this.location <- this.inherit("scripts/entity/world/world_entity", {
			m = {},
			function onInit() {
				this.world_entity.onInit();
				this.setRenderedTop(false);
				this.setVisibleInFogOfWar(true);
				this.World.EntityManager.addLocation(this);
				this.getTile().IsOccupied = true;
				local label = this.addLabel("name");
				label.Visible = false;
				label.Text = this.getName();
			}
		});
		"#
		.into(),
	);
	m.insert(
		"scripts/entity/world/locations/test_location".into(),
		r#"this.test_location <- this.inherit("scripts/entity/world/location", { m = {} });"#
			.into(),
	);
	MemSource(m)
}

#[test]
fn spawn_location_instantiates_registers_and_binds_tile() {
	let engine = Engine::new(sources());

	// Stand up the grid + the (script) EntityManager — what world_state does before
	// worldgen in the real game.
	engine
		.run_script(
			r#"
			this.World.resizeScene(8, 8);
			this.World.EntityManager <- this.new("scripts/entity/world/entity_manager");
			"#,
			"setup",
		)
		.expect("grid + entity manager set up");

	engine
		.run_script(
			r#"
			// tile.Coords is axial {X=q, Y=r}; spawn the location there.
			local tile = this.World.getTileSquare(3, 3);
			local entity = this.World.spawnLocation(
				"scripts/entity/world/locations/test_location", tile.Coords);

			assert(entity.getID() == 1);                                  // monotonic id
			assert(this.World.EntityManager.getLocations().len() == 1);   // registered
			assert(this.World.getTileSquare(3, 3).IsOccupied == true);    // tile bound
			assert(entity.getTile().isSameTileAs(this.World.getTileSquare(3, 3)));
			assert(entity.hasLabel("name"));                             // label attached
			assert(entity.getLabel("name").Text == "Test Site");        // and live
			assert(entity.hasSprite("body"));                           // sprite attached (onAfterInit)
			"#,
			"spawn",
		)
		.expect("spawnLocation runs the full light init path");

	assert_eq!(engine.world_entities().count(), 1);
}
