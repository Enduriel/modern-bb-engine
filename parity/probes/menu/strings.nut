// mode: menu
// BB string natives and the Squirrel string stdlib scripts and mods rely on.
local function p( _k, _f )
{
	local v;
	try { v = _f(); } catch (e) { v = "ERROR " + e; }
	::logInfo("PROBE " + _k + " = " + typeof v + " " + v);
}
p("buildTextFromTemplate", @() ::buildTextFromTemplate("Hi %name%, %n% gold", [["name", "Bob"], ["n", 12]]));
p("isFirstCharacter yes", @() ::isFirstCharacter("Apple", "A"));
p("isFirstCharacter no", @() ::isFirstCharacter("Apple", "p"));
p("removeFromBeginningOfText", @() ::removeFromBeginningOfText("abc", "abcdef"));
p("hexStringToInt ff", @() ::hexStringToInt("ff"));
p("hexStringToInt #1A2b", @() ::hexStringToInt("#1A2b"));
p("String.contains", @() ::String.contains("hello world", "o w"));
p("String.replace", @() ::String.replace("a-b-c", "-", "+"));
p("String.remove", @() ::String.remove("a-b-c", "-"));
p("format %d %s %.2f", @() ::format("%d %s %.2f", 42, "x", 3.14159));
p("format %5.1f|%-4d|", @() ::format("%5.1f|%-4d|", 2.25, 7));
p("strip", @() ::strip("  a b  ") + "|");
p("lstrip", @() ::lstrip("  a ") + "|");
p("rstrip", @() ::rstrip(" a  ") + "|");
p("tointeger 12abc", @() "12abc".tointeger());
p("tointeger 0x1F", @() "0x1F".tointeger());
p("tofloat 2.50", @() "2.50".tofloat());
p("tofloat 1e3", @() "1e3".tofloat());
p("slice", @() "abcdef".slice(1, -1));
p("find", @() "abcabc".find("c", 3));
p("toupper", @() "aBc".toupper());
p("len", @() "héllo".len());
p("regexp match", @() ::regexp("^\\w+_(\\d+)$").match("__sqrat_ol_ rand_2"));
p("regexp capture", @() ::regexp("(\\d+)").capture("ab123") != null);
