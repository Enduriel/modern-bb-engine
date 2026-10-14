// mode: menu
// The class system's object layout: what an instance made by ::new looks
// like level by level, for a skill (brave_trait -> character_trait -> skill) and an item
// (arming_sword -> weapon -> item): own slots, the super slots, the delegate chain,
// whether methods are copied and bound per instance, and the member tables `m`.
local function p( _k, _f )
{
	local v;
	try { v = _f(); } catch (e) { v = "ERROR " + e; }
	::logInfo("PROBE " + _k + " = " + typeof v + " " + v);
}

local function ownCounts( _t )
{
	local fn = 0, other = 0;
	foreach( k, v in _t )
	{
		if (typeof v == "function" || typeof v == "native function") fn++;
		else other++;
	}
	return fn + " functions, " + other + " other";
}

local function ownKeys( _t, _kind )
{
	local keys = [];
	foreach( k, v in _t )
	{
		local isFn = typeof v == "function" || typeof v == "native function";
		if (_kind == "data" ? !isFn : isFn) keys.push(k);
	}
	keys.sort();
	return keys.len() == 0 ? "-" : keys.reduce(@(a, b) a + " " + b);
}

local function layout( _label, _path, _levels, _methods )
{
	local a = ::new(_path), b = ::new(_path);
	// The level tables of `a`, leaf first, through the super slots.
	local lv = [a];
	foreach( name in _levels ) lv.push(lv[lv.len() - 1][name]);
	local names = ["leaf"];
	names.extend(_levels);

	foreach( i, t in lv )
	{
		local n = _label + " " + names[i];
		p(n + " typeof", @() typeof t);
		p(n + " own slots", @() ownCounts(t));
		p(n + " own data keys", @() ownKeys(t, "data"));
		p(n + " delegate is next level", @() i + 1 < lv.len() ? t.getdelegate() == lv[i + 1] : t.getdelegate() == null);
		p(n + " delegate typeof", @() typeof t.getdelegate());
		p(n + " m own", @() t.rawin("m"));
		p(n + " m is leaf m", @() t.m == a.m);
		p(n + " m own slots", @() ownCounts(t.m));
		p(n + " m delegate is next level m", @() i + 1 < lv.len() ? t.m.getdelegate() == lv[i + 1].m : t.m.getdelegate() == null);
		foreach( meth in _methods )
		{
			p(n + " rawin " + meth, @() t.rawin(meth));
		}
	}
	p(_label + " super slot per instance", @() a[_levels[0]] != b[_levels[0]]);
	p(_label + " delegate chain length", function() {
		local n = 0;
		for( local t = a; t != null && n < 50; t = t.getdelegate() ) n++;
		return n;
	});

	foreach( meth in _methods )
	{
		p(_label + " " + meth + " same object per fetch", @() a[meth] == a[meth]);
		p(_label + " " + meth + " shared by instances", @() a[meth] == b[meth]);
		p(_label + " " + meth + " via super same as via leaf", @() a[_levels[0]][meth] == a[meth]);
	}

	// Which object does a method fetched from `a` run on when called with `b`?
	a.m.ID = "A-id";
	b.m.ID = "B-id";
	p(_label + " getID fetched from leaf, called on other", @() a.getID.call(b));
	p(_label + " getID fetched from super, called on other", @() a[_levels[0]].getID.call(b));
	p(_label + " getID via super reads leaf", @() a[_levels[0]].getID());
	p(_label + " super m.ID after leaf write", @() a[_levels[0]].m.ID);
	p(_label + " m.ID own on leaf m after write", @() a.m.rawin("ID"));
	local f = a.getID;
	p(_label + " getID detached, plain call", @() f());

	// The extra slots the original puts on an instance, and its self/base natives.
	local baseLevel = lv[lv.len() - 1];
	local baseB = b;
	foreach( name in _levels ) baseB = baseB[name];
	p(_label + " release hook typeof", @() typeof a._release_hook_DO_NOT_delete_it_);
	p(_label + " release hook shared by instances", @() a._release_hook_DO_NOT_delete_it_ == b._release_hook_DO_NOT_delete_it_);
	p(_label + " InstanceName", @() baseLevel.InstanceName);
	p(_label + " InstanceName on leaf", @() a.InstanceName);
	foreach( nat in ["self", "base"] )
	{
		p(_label + " " + nat + " typeof", @() typeof baseLevel[nat]);
		p(_label + " " + nat + " native", @() baseLevel[nat].getinfos().native);
		p(_label + " " + nat + " own on leaf", @() a.rawin(nat));
		p(_label + " " + nat + " shared by instances", @() baseLevel[nat] == baseB[nat]);
	}
	// self() and base() are only checked as slots: calling base() on an instance crashes
	// the original, and so does `typeof` of what self() returns.
	local c = ::new(_path);
	p(_label + " InstanceName of a, b, c", @() a.InstanceName + " " + b.InstanceName + " " + c.InstanceName);
}

layout("skill", "scripts/skills/traits/brave_trait", ["character_trait", "skill"], ["create", "getID", "getName", "onUpdate", "getTooltip", "isType", "onAdded"]);
layout("item", "scripts/items/weapons/arming_sword", ["weapon", "item"], ["create", "getID", "getName", "getValue", "getTooltip", "onEquip", "isItemType"]);

// InstanceName's counter: per class, or one across classes?
p("InstanceName counter across classes", function() {
	local x = ::new("scripts/skills/traits/brave_trait"), y = ::new("scripts/items/weapons/arming_sword"), z = ::new("scripts/skills/traits/brave_trait");
	return x.InstanceName + " " + y.InstanceName + " " + z.InstanceName;
});
