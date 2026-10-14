// mode: campaign seed=rust-engine-test
// World.getNavigator().findPath routes under the settings variants world scripts use.
local function p( _k, _f )
{
	local v;
	try { v = _f(); } catch (e) { v = "ERROR " + e; }
	::logInfo("PROBE " + _k + " = " + typeof v + " " + v);
}
local nav = ::World.getNavigator();
local settlements = ::World.EntityManager.getSettlements();
local function route( _from, _to, _settings, _maxLen )
{
	local path = nav.findPath(_from, _to, _settings, _maxLen);
	if (path.isEmpty()) return "empty";
	local n = path.getSize(), h = 0;
	while (path.getSize() >= 1)
	{
		local c = path.getCurrent();
		h = (h * 31 + c.X * 1000 + c.Y) % 1000003;
		path.pop();
	}
	return n + " h" + h;
}
local variants = [
	["default", function( s ) {}],
	["terrain", function( s ) { s.ActionPointCosts = ::Const.World.TerrainTypeNavCost; }],
	["terrain road1/1.5", function( s ) { s.ActionPointCosts = ::Const.World.TerrainTypeNavCost; s.RoadMult = 1.0 / 1.5; }],
	["terrain road*0.05 heat100", function( s ) { s.ActionPointCosts = ::Const.World.TerrainTypeNavCost; s.RoadMult = 1.0 / 1.5 * 0.05; s.HeatCost = 100; }],
	["terrain roadonly", function( s ) { s.ActionPointCosts = ::Const.World.TerrainTypeNavCost; s.RoadOnly = true; }],
	["terrain stopatroad", function( s ) { s.ActionPointCosts = ::Const.World.TerrainTypeNavCost; s.StopAtRoad = true; }],
	["sneak heat100", function( s ) { s.ActionPointCosts = ::Const.World.TerrainTypeNavCost_Sneak; s.RoadMult = 1.0; s.HeatCost = 100; }],
	["flat", function( s ) { s.ActionPointCosts = ::Const.World.TerrainTypeNavCost_Flat; }],
];
local pairs = [[0, 5], [3, 12], [7, 1], [10, 20], [15, 4]];
foreach (v in variants)
	foreach (pr in pairs)
		p("route " + v[0] + " s" + pr[0] + "->s" + pr[1], function() {
			local s = nav.createSettings();
			v[1](s);
			return route(settlements[pr[0]].getTile(), settlements[pr[1]].getTile(), s, 0);
		});
foreach (maxLen in [1, 2, 3, 6])
	p("route terrain maxLen" + maxLen + " s0->s5", function() {
		local s = nav.createSettings();
		s.ActionPointCosts = ::Const.World.TerrainTypeNavCost;
		return route(settlements[0].getTile(), settlements[5].getTile(), s, maxLen);
	});
p("route same tile", function() { local t = settlements[0].getTile(); return route(t, t, nav.createSettings(), 0); });
