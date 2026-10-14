// mode: menu
// The order `foreach` visits a table's slots in, which follows from where each key
// lands, i.e. from the string hash (Squirrel 3.0's in the original). Scripts depend on
// it: Modern Hooks registers the DLCs by iterating ::Const.DLC. Short keys, keys of 32
// characters and more (the hash then skips characters), integer keys, growth and
// removal.
local function p( _k, _f )
{
	local v;
	try { v = _f(); } catch (e) { v = "ERROR " + e; }
	::logInfo("PROBE " + _k + " = " + typeof v + " " + v);
}

local function keys( _t )
{
	local s = "";
	foreach (k, v in _t) s += (s == "" ? "" : ",") + k;
	return s;
}

p("Const.DLC", @() keys(::Const.DLC));
p("Const.Faction", @() keys(::Const.Faction));
p("literal", @() keys({ Alpha = 1, Beta = 2, Gamma = 3, Delta = 4, Epsilon = 5, Zeta = 6, Eta = 7 }));
p("grown one by one", function() {
	local t = {};
	foreach (k in ["Hitpoints", "Bravery", "Stamina", "MeleeSkill", "RangedSkill", "MeleeDefense", "RangedDefense", "Initiative", "Fatigue", "ActionPoints", "Armor", "Morale", "Level", "XP", "Name", "Title", "Background", "Talents"])
		t[k] <- 0;
	return keys(t);
});
p("long keys", function() {
	local t = {};
	foreach (k in ["scripts/skills/perks/perk_quick_hands", "scripts/skills/perks/perk_mastery_swordc", "scripts/items/weapons/named/named_two_handed_mace", "scripts/entity/tactical/humans/bandit_raider", "scripts/entity/world/settlements/small_lumber_village"])
		t[k] <- 0;
	return keys(t);
});
p("integer keys", function() {
	local t = {};
	foreach (k in [17, 3, 250, -4, 1024, 99, 0, 65])
		t[k] <- 0;
	return keys(t);
});
p("after removals", function() {
	local t = {};
	foreach (k in ["a", "bb", "ccc", "dddd", "eeeee", "ffffff", "ggggggg", "hhhhhhhh", "iiiiiiiii"])
		t[k] <- 0;
	foreach (k in ["bb", "dddd", "ffffff", "hhhhhhhh"])
		delete t[k];
	t.zz <- 1;
	return keys(t);
});
