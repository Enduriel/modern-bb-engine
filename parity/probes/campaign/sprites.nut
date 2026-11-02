// mode: campaign seed=rust-engine-test
// An entity's sprites by name: adding a name it already has hands back the sprite
// there and logs a warning (`Entity`); removing one drops it. Battle units (a brother
// in the temporary roster) and world entities (the player's party) share the natives.
// Leaves the world as it found it.
local function p( _k, _f )
{
	local v;
	try { v = _f(); } catch (e) { v = "ERROR " + e; }
	::logInfo("PROBE " + _k + " = " + typeof v + " " + v);
}
local roster = ::World.getTemporaryRoster();
roster.clear();
local t = roster.create("scripts/entity/tactical/human");
local party = ::World.State.getPlayer();

foreach (who in [["unit", t], ["party", party]])
{
	local k = who[0], e = who[1];
	p(k + " has new", @() e.hasSprite("bb_probe_dup"));
	::logInfo("PROBE_LOG " + k + " add new");
	p(k + " add new", function() { e.addSprite("bb_probe_dup").Saturation = 0.25; return e.hasSprite("bb_probe_dup"); });
	::logInfo("PROBE_LOG " + k + " add again");
	p(k + " add again", @() e.addSprite("bb_probe_dup").Saturation);
	::logInfo("PROBE_LOG " + k + " insert again");
	p(k + " insert again", @() e.insertSprite("bb_probe_dup").Saturation);
	::logInfo("PROBE_LOG " + k + " remove");
	p(k + " remove", function() { e.removeSprite("bb_probe_dup"); return e.hasSprite("bb_probe_dup"); });
	::logInfo("PROBE_LOG " + k + " remove missing");
	p(k + " remove missing", function() { e.removeSprite("bb_probe_never_added"); return e.hasSprite("bb_probe_never_added"); });
	::logInfo("PROBE_LOG " + k + " add after remove");
	p(k + " add after remove", @() e.addSprite("bb_probe_dup").Saturation);
	e.removeSprite("bb_probe_dup");
}
roster.clear();
