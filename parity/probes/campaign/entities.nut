// mode: campaign seed=rust-engine-test
// World entities right after the new campaign starts (the world is paused).
local function p( _k, _f )
{
	local v;
	try { v = _f(); } catch (e) { v = "ERROR " + e; }
	::logInfo("PROBE " + _k + " = " + typeof v + " " + v);
}
local settlements = ::World.EntityManager.getSettlements();
p("settlement count", @() settlements.len());
foreach (i, s in settlements)
	if (i % 4 == 0)
		p("settlement " + i, @() s.getName() + " @" + s.getTile().SquareCoords.X + "," + s.getTile().SquareCoords.Y);
// Entity IDs come from a global object counter in the original, so only their order and
// spacing are comparable: the distinct gaps between consecutive settlements.
p("settlement id gaps", function() {
	local gaps = [];
	for (local i = 1; i < settlements.len(); i++) {
		local g = settlements[i].getID() - settlements[i - 1].getID();
		if (gaps.find(g) == null) gaps.push(g);
	}
	gaps.sort();
	return gaps.reduce(@(a, b) a + " " + b);
});
p("settlement ids ascending", function() {
	for (local i = 1; i < settlements.len(); i++) if (settlements[i].getID() <= settlements[i - 1].getID()) return false;
	return true;
});
local player = ::World.State.getPlayer();
p("player tile", @() player.getTile().SquareCoords.X + "," + player.getTile().SquareCoords.Y);
p("player pos", @() player.getPos().X + "," + player.getPos().Y);
p("entities at player r500", @() ::World.getAllEntitiesAtPos(player.getPos(), 500.0).len());
p("entities at player r2000", @() ::World.getAllEntitiesAtPos(player.getPos(), 2000.0).len());
p("entity by id", @() ::World.getEntityByID(settlements[3].getID()).getName());
p("player entity is player", @() ::World.getPlayerEntity() == player);
p("factions", @() ::World.FactionManager.getFactions().len());
p("money", @() ::World.Assets.getMoney());
p("roster size", @() ::World.getPlayerRoster().getSize());
