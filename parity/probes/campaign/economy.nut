// mode: campaign seed=bbuitest
// Every town's economy on a new campaign, from the play session's seed: each town's
// buildings, its shop stock with buy and sell prices, and its recruits with their
// costs and wages (both from pinned seeds: settlement.create seeds them with the real
// time); the tavern's prices, the company's wages and training costs, the temple's
// price for four injuries, and the followers' costs and locks. It restocks every town:
// the group's other probes see that.
local function p( _k, _f )
{
	local v;
	try { v = _f(); } catch (e) { v = "ERROR " + e; }
	::logInfo("PROBE " + _k + " = " + typeof v + " " + v);
}
local towns = ::World.EntityManager.getSettlements();
p("towns", function() { return towns.len(); });
foreach (i, town in towns)
{
	local t = "t" + (i < 10 ? "0" : "") + i;
	p(t, function() {
		local b = [];
		foreach (x in town.m.Buildings) if (x != null) b.push(x.getID());
		return town.getName() + " size " + town.getSize() + " buy " + town.getBuyPriceMult() + " sell " + town.getSellPriceMult() + " [" + b.reduce(@(a, c) a + " " + c) + "]";
	});
	::World.State.m.LastEnteredTown = ::WeakTableRef(town);
	// The seeds hold the real time at creation (settlement.create): pin them, so both
	// engines generate from the same ones.
	town.m.ShopSeed = 1000 + i;
	town.m.RosterSeed = 2000 + i;
	p(t + " shop", function() { town.updateShop(true); return "ok"; });
	foreach (x in town.m.Buildings)
	{
		if (x == null || x.getStash() == null) continue;
		p(t + " " + x.getID(), function() {
			local s = "";
			foreach (it in x.getStash().getItems()) if (it != null) s += it.getID() + " " + it.getBuyPrice() + "/" + it.getSellPrice() + "; ";
			return s;
		});
	}
	p(t + " roster", function() { town.updateRoster(true); return "ok"; });
	p(t + " recruits", function() {
		local s = "";
		foreach (r in ::World.getRoster(town.getID()).getAll())
			s += r.getBackground().getID() + " L" + r.getLevel() + " hire " + r.getHiringCost() + " wage " + r.getDailyCost() + " tryout " + r.getTryoutCost() + "; ";
		return s;
	});
	if (town.hasBuilding("building.tavern"))
		p(t + " tavern", function() { local v = town.getBuilding("building.tavern"); return "rumor " + v.getRumorPrice() + " drink " + v.getDrinkPrice(); });
	::World.State.m.LastEnteredTown = null;
}
p("bros", function() {
	local s = "";
	foreach (b in ::World.getPlayerRoster().getAll()) s += b.getBackground().getID() + " L" + b.getLevel() + " wage " + b.getDailyCost() + " train " + (80 + 50 * b.getLevel()) + "/" + (100 + 60 * b.getLevel()) + "/" + (90 + 55 * b.getLevel()) + "; ";
	return s;
});
p("injuries", function() {
	::World.State.m.LastEnteredTown = ::WeakTableRef(towns[0]);
	local b = ::World.getPlayerRoster().getAll()[0], s = "";
	foreach (n in ["cut_arm_injury", "broken_nose_injury", "broken_ribs_injury", "bruised_leg_injury"])
	{
		local inj = ::new("scripts/skills/injury/" + n);
		b.getSkills().add(inj);
		s += n + " " + inj.getPrice() + "; ";
		b.getSkills().remove(inj);
	}
	::World.State.m.LastEnteredTown = null;
	return s;
});
p("followers", function() {
	local s = "";
	foreach (f in ::World.Retinue.m.Followers) s += f.getID() + " cost " + f.getCost() + " unlocked " + f.isUnlocked() + "; ";
	return s;
});
p("assets", function() { return "money " + ::World.Assets.getMoney() + " renown " + ::World.Assets.getBusinessReputation(); });
