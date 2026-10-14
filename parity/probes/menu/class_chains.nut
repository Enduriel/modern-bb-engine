// mode: menu
// How `inherit` lays out a class's ancestors: each class gets its own linked copy
// of its ancestors' levels in its super slot, and the class table has no delegate, so a
// write through one class's ancestor levels (Modern Hooks' `q.m.X = v`) stays with that
// class. Two classes are made here from a compiled string, both inheriting
// `scripts/items/weapons/weapon`.
local function p( _k, _f )
{
	local v;
	try { v = _f(); } catch (e) { v = "ERROR " + e; }
	::logInfo("PROBE " + _k + " = " + typeof v + " " + v);
}

local scope = {};
::compilestring("this.chain_a <- ::inherit(\"scripts/items/weapons/weapon\", { m = { Own = 1 }, function create() { this.weapon.create(); } }); this.chain_b <- ::inherit(\"scripts/items/weapons/weapon\", { m = {}, function create() { this.weapon.create(); } });").call(scope);
local a = scope.chain_a, b = scope.chain_b;

p("class table has no delegate", @() a.getdelegate() == null);
p("class m has no delegate", @() a.m.getdelegate() == null);
p("SuperName", @() a.SuperName);
p("super slot is a table", @() typeof a.weapon);
p("own copy of the parent level", @() a.weapon != b.weapon);
p("own copy of the grandparent level", @() a.weapon.item != b.weapon.item);
p("copied parent level delegates to the copied grandparent", @() a.weapon.getdelegate() == a.weapon.item);
p("copied parent m delegates to the copied grandparent m", @() a.weapon.m.getdelegate() == a.weapon.item.m);
p("copied grandparent level has no delegate", @() a.weapon.item.getdelegate() == null);
p("copied parent m own slots", function() { local n = 0; foreach (k, v in a.weapon.m) n++; return n; });
p("copied parent keeps its methods", @() typeof a.weapon.getAmmoMax);
p("write through one class's copy", function() {
	a.weapon.item.m.Condition = 4321;
	return a.weapon.item.m.Condition + " / " + b.weapon.item.m.Condition;
});
