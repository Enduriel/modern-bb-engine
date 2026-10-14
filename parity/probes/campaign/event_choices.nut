// mode: campaign seed=eventchoices
// Events' other choices: a dozen recruits of the backgrounds events look for join (pinned
// seeds), with crowns and a few items, and every event that then applies runs with
// pinned seeds, answered on each screen with its LAST option (often the way out, or the
// other choice) until it ends: the screens it went through and the company after.
// A group of its own: events just run (as in `events_company`) are on cooldown.
local function p( _k, _f )
{
	local v;
	try { v = _f(); } catch (e) { v = "ERROR " + e; }
	::logInfo("PROBE " + _k + " = " + typeof v + " " + v);
}
foreach (i, bg in ["butcher_background", "brawler_background", "cultist_background", "flagellant_background", "hunter_background",
	"thief_background", "monk_background", "minstrel_background", "squire_background", "sellsword_background",
	"historian_background", "wildman_background", "gambler_background", "killer_on_the_run_background"])
{
	p("recruit " + i, function() {
		::Math.seedRandom(300 + i);
		local b = ::World.getPlayerRoster().create("scripts/entity/tactical/player");
		b.setStartValuesEx([bg]);
		return b.getBackground().getID() + " L" + b.getLevel() + " wage " + b.getDailyCost();
	});
}
::World.Assets.addMoney(5000);
foreach (it in ["scripts/items/accessory/wardog_item", "scripts/items/misc/witch_hair_item", "scripts/items/supplies/wine_item"])
	::World.Assets.getStash().add(::new(it));
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
			local last = e.m.ActiveScreen == null ? 0 : e.m.ActiveScreen.Options.len() - 1;
			if (!e.processInput(last < 0 ? 0 : last)) break;
			steps++;
		}
		e.clear();
		::World.Events.m.ActiveEvent = null;
		return "score " + score + " " + trail + " " + company();
	});
}
p("end", company);
