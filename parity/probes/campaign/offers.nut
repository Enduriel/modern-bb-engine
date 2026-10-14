// mode: campaign seed=bbuitest
// The contracts the factions offer on a new campaign: every faction's contract actions
// (scripts/factions/contracts), each scored and, where it applies, run with pinned
// seeds as the faction runs it; the contract it adds is described (type, home, target,
// difficulty, payment) and taken off again. Their actions also change the towns (the
// situations they add move prices): keep this probe last in the group.
local function p( _k, _f )
{
	local v;
	try { v = _f(); } catch (e) { v = "ERROR " + e; }
	::logInfo("PROBE " + _k + " = " + typeof v + " " + v);
}
local actions = [];
foreach (f in ::IO.enumerateFiles("scripts/factions/contracts/"))
{
	local name = f.slice(f.find("contracts/") + 10);
	actions.push(name);
}
p("contract actions", @() actions.len());
local factions = ::World.FactionManager.getFactions();
local seed = 0;
foreach (fi, f in factions)
{
	if (f == null) continue;
	foreach (a in f.m.Deck)
	{
		local path = a.ClassName;
		if (actions.find(path) == null) continue;
		seed++;
		::Math.seedRandom(5000 + seed);
		local score = 0;
		try { a.update(true); score = a.getScore(); } catch (e) { score = "ERROR " + e; }
		if (score == 0) continue;
		p("f" + fi + " " + path, function() {
			local before = ::World.Contracts.getOpenContracts().len();
			::Math.seedRandom(6000 + seed);
			a.execute(true);
			local open = ::World.Contracts.getOpenContracts();
			if (open.len() == before) return "score " + score + " no contract";
			local c = open[open.len() - 1];
			local pay = c.getPayment();
			local s = "score " + score + " " + c.getType() + " home " + (c.getHome() == null ? "-" : c.getHome().getName())
				+ " difficulty " + c.getDifficultyMult() + " paymult " + c.getPaymentMult()
				+ " pool " + pay.Pool + " advance " + pay.getInAdvance() + " completion " + pay.getOnCompletion() + " per " + pay.getPerCount();
			if ("Destination" in c.m && c.m.Destination != null && !c.m.Destination.isNull()) s += " to " + c.m.Destination.getName();
			::World.Contracts.removeContract(c);
			return s;
		});
	}
}
p("open contracts after", @() ::World.Contracts.getOpenContracts().len());
