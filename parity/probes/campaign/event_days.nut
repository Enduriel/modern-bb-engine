// mode: campaign seed=eventdays
// Events later in a campaign: day 40. Nothing sets the world's clock (a save restores
// it natively), so `World.getTime` is wrapped to report 39 days on (`Days`, `Time`; the
// rest as it is), in both engines alike, and put back at the end. Then as
// `events_company`: a dozen recruits of the backgrounds events look for (pinned seeds),
// crowns and a few items, and every event that then applies run with pinned seeds and
// its first options: the screens and the company after each. 42 events read
// `World.getTime().Days`.
local function p( _k, _f )
{
	local v;
	try { v = _f(); } catch (e) { v = "ERROR " + e; }
	::logInfo("PROBE " + _k + " = " + typeof v + " " + v);
}
local realGetTime = ::World.getTime;
::World.getTime = function()
{
	local t = realGetTime.call(::World);
	return {
		Time = t.Time + 39 * t.SecondsPerDay, Days = t.Days + 39,
		SecondsPerDay = t.SecondsPerDay, SecondsPerHour = t.SecondsPerHour, SecondsPerMinute = t.SecondsPerMinute,
		SecondsOfDay = t.SecondsOfDay, Hours = t.Hours, Minutes = t.Minutes,
		TimeOfDay = t.TimeOfDay, IsDaytime = t.IsDaytime, IsPaused = t.IsPaused
	};
};
p("clock", function() {
	local t = ::World.getTime();
	return "day " + t.Days + " hour " + t.Hours + " tod " + t.TimeOfDay + " daytime " + t.IsDaytime;
});
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
			if (!e.processInput(0)) break;
			steps++;
		}
		e.clear();
		::World.Events.m.ActiveEvent = null;
		return "score " + score + " " + trail + " " + company();
	});
}
p("end", company);
::World.getTime = realGetTime;
