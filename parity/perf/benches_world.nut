// Performance baselines, world part: see benches.nut. Needs the new
// campaign's world (seed rust-engine-test), and leaves it unchanged.

::PerfWorldBenches <- [];

// The world navigator: long routes between settlements, as parties plan them.
::PerfWorldBenches.push({
	Name = "world_paths",
	function Run()
	{
		local nav = ::World.getNavigator();
		local settlements = ::World.EntityManager.getSettlements();
		local n = 0;

		foreach( i, from in settlements )
		{
			local to = settlements[(i * 7 + 5) % settlements.len()];
			local settings = nav.createSettings();
			settings.ActionPointCosts = ::Const.World.TerrainTypeNavCost;
			settings.RoadMult = 1.0 / 1.5;
			n += nav.findPath(from.getTile(), to.getTile(), settings, 0).getSize();
		}

		return n;
	}

});
// Tile access: every tile of the map, by square coordinates.
::PerfWorldBenches.push({
	Name = "world_tiles",
	function Run()
	{
		local size = ::World.getMapSize();
		local n = 0;

		for( local x = 0; x < size.X; x++ )
		{
			for( local y = 0; y < size.Y; y++ )
			{
				n += ::World.getTileSquare(x, y).Type;
			}
		}

		return n;
	}

});
// Spatial queries: entities around each settlement.
::PerfWorldBenches.push({
	Name = "world_entity_queries",
	function Run()
	{
		local n = 0;

		for( local r = 0; r < 20; r++ )
		{
			foreach( s in ::World.EntityManager.getSettlements() )
			{
				n += ::World.getAllEntitiesAtPos(s.getPos(), 1000.0).len();
			}
		}

		return n;
	}

});
