// mode: campaign seed=bbuitest
// Every event that applies to a new campaign, run in the order the game lists them
// (IO.enumerateFiles) with pinned seeds and answered with its first option until it
// ends: the screens it went through and the company after. The events run directly
// (fire, processInput, clear), without the event screen. Changes the company: keep it
// last in the group.
local function p( _k, _f )
{
	local v;
	try { v = _f(); } catch (e) { v = "ERROR " + e; }
	::logInfo("PROBE " + _k + " = " + typeof v + " " + v);
}
local function company()
{
	local a = ::World.Assets;
	local s = "money " + a.getMoney() + " food " + a.getFood() + " ammo " + a.getAmmo() + " parts " + a.getArmorParts() + " meds " + a.getMedicine() + " renown " + a.getBusinessReputation() + " moral " + a.getMoralReputation();
	foreach (b in ::World.getPlayerRoster().getAll())
	{
		local n = 0;
		foreach (sk in b.getSkills().m.Skills) if (sk.isType(::Const.SkillType.Trait) || sk.isType(::Const.SkillType.TemporaryInjury) || sk.isType(::Const.SkillType.StatusEffect)) n++;
		s += " | " + b.getName() + " " + b.getBackground().getID() + " mood " + b.getMood() + " xp " + b.getXP() + " hp " + b.getHitpoints() + " skills " + n;
	}
	local n = 0, ids = "";
	foreach (it in a.getStash().getItems()) if (it != null) { n++; ids += " " + it.getID(); }
	return s + " | stash " + n + ids;
}
local events = ::World.Events.m.Events;
p("count", function() { return events.len(); });
p("order", function() { local s = ""; foreach (e in events) s += e.getID() + " "; return s; });
foreach (i, e in events)
{
	local id = e.getID();
	::Math.seedRandom(1000 + i);
	local score = 0;
	try { e.update(); score = e.getScore(); } catch (err) { score = "ERROR " + err; }
	if (score == 0) continue;
	p("e" + i + " " + id, function() {
		::Math.seedRandom(2000 + i);
		::World.Events.m.ActiveEvent = e;
		e.fire();
		local trail = "", steps = 0;
		while (steps < 12)
		{
			trail += (e.m.ActiveScreen == null ? "?" : e.m.ActiveScreen.ID) + " ";
			if (!e.processInput(0)) break;
			steps++;
		}
		e.clear();
		::World.Events.m.ActiveEvent = null;
		return "score " + score + " " + trail + " " + company();
	});
}
p("end", company);
