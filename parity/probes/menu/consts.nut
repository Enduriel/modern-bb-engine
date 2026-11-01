// mode: menu
// The engine's enums in the const table (getconsttable()): their names and values, and
// that script code compiled after startup resolves them at compile time.
local function p( _k, _f )
{
	local v;
	try { v = _f(); } catch (e) { v = "ERROR " + e; }
	::logInfo("PROBE " + _k + " = " + typeof v + " " + v);
}
local function sortedKeys( _t )
{
	local keys = [];
	foreach (k, v in _t) keys.push(k);
	keys.sort();
	return keys;
}
local ct = ::getconsttable();
p("names", function() { local s = ""; foreach (k in sortedKeys(ct)) s += k + ","; return s; });
foreach (name in sortedKeys(ct))
{
	local e = ct[name];
	if (typeof e != "table") { p("const " + name, @() e); continue; }
	foreach (k in sortedKeys(e)) p(name + "." + k, @() e[k]);
}
p("KeyModifier is KeyState", @() ct.KeyModifier == ct.KeyState);
p("Key in root", @() "Key" in ::getroottable());
p("KeyModifier in root", @() "KeyModifier" in ::getroottable());
// A const wins over a root slot of the same name: the compiler substitutes the value.
p("compiled KeyModifier.Control", function()
{
	::getroottable().KeyModifier <- { Control = 99 };
	local v = ::compilestring("return KeyModifier.Control;")();
	delete ::getroottable().KeyModifier;
	return v;
});
p("compiled Key.A", @() ::compilestring("return Key.A;")());
p("compiled Mouse.Wheel", @() ::compilestring("return Mouse.Wheel;")());
p("compiled ShakeType.KnockUp", @() ::compilestring("return ShakeType.KnockUp;")());
