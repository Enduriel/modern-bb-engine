// mode: menu
// new() and inherit() of a script that doesn't exist: the original logs the failure and
// gives null (it doesn't throw); using the null fails later.
local function p( _k, _f )
{
	local v;
	try { v = _f(); } catch (e) { v = "ERROR " + e; }
	::logInfo("PROBE " + _k + " = " + typeof v + " " + v);
}
p("new missing", function() { local x = ::new("scripts/skills/backgrounds/background.nope"); return x == null ? "null" : typeof x; });
p("this.new missing", function() { local x = this.new("scripts/nope/missing_thing"); return x == null ? "null" : typeof x; });
p("new missing then use", function() { local x = ::new("scripts/nope/missing_thing"); return x.getID(); });
p("inherit missing", function() { local c = ::inherit("scripts/nope/missing_base", {}); return c == null ? "null" : typeof c; });
