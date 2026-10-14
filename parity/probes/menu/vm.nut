// mode: menu
// Squirrel VM semantics: number formatting, integer width, operators, errors.
local function p( _k, _f )
{
	local v;
	try { v = _f(); } catch (e) { v = "ERROR " + e; }
	::logInfo("PROBE " + _k + " = " + typeof v + " " + v);
}
p("tostring 0.1", @() "" + 0.1);
p("tostring 1.0/3", @() "" + (1.0 / 3));
p("tostring 2.0", @() "" + 2.0);
p("tostring 1e10", @() "" + 1e10);
p("tostring 123456789.0", @() "" + 123456789.0);
p("tostring 1e-7", @() "" + 1e-7);
p("tostring 170*0.7", @() "" + (170 * 0.7));
p("tointeger 170*0.7", @() (170 * 0.7).tointeger());
p("int max plus 1", @() 2147483647 + 1);
p("int 1 shl 31", @() 1 << 31);
p("int -1 shr 1", @() -1 >> 1);
p("int -1 ushr 1", @() -1 >>> 1);
p("int ~0", @() ~0);
p("int 7/2", @() 7 / 2);
p("int -7/2", @() -7 / 2);
p("int -7%3", @() -7 % 3);
p("float 7%2.5", @() 7 % 2.5);
p("div by zero", @() 1 / 0);
p("float div by zero", @() 1.0 / 0);
p("cmp spaceship 1,2", @() 1 <=> 2);
p("cmp str a,b", @() "a" < "b");
p("cmp int float", @() 1 == 1.0);
p("call non-callable", @() (5)());
// Not probed: compiling `base++` crashes the original (its 3.0.4 compiler); our 3.2 VM raises a compile error.
p("typeof null", @() typeof null);
p("array sort", function() { local a = [3, 1, 2, 1]; a.sort(); local s = ""; foreach (x in a) s += x; return s; });
p("table order", function() { local t = { b = 1, a = 2, c = 3, d = 4 }; local s = ""; foreach (k, v in t) s += k; return s; });
p("missing index", function() { local t = {}; return t.nope; });
p("null index", function() { local t = null; return t.x; });
