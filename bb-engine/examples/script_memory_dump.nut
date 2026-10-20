// Every table and array reachable from the root, in a stable text form: each table's slots
// in its own iteration order (sorted when a key is an object, whose order follows
// addresses), each value by kind; closures by name and source. Lines go to ::__dump.
::__dump <- [];
local seen = {};
local queue = [[getroottable(), "root"]];
local function desc(v)
{
	switch (typeof v)
	{
		case "function":
			local i = v.getinfos();
			return "fn " + i.name + "@" + ("src" in i ? i.src : "native");
		case "native function":
			local i = v.getinfos();
			return "nfn " + i.name;
		case "table": case "array": case "instance": case "class": case "userdata": case "weakref": case "generator": case "thread":
			return typeof v;
		case "string":
			return "s" + v.len() + ":" + (v.len() > 40 ? v.slice(0, 40) : v);
		case "float":
			return "f" + v;
		case "integer":
			// Ids made from addresses (toHash) differ from run to run.
			return v > 0x10000000000 || v < -0x10000000000 ? "address" : "i" + v;
		case "bool": case "null":
			return typeof v + v;
		default:
			return typeof v;
	}
}
local qi = 0;
while (qi < queue.len())
{
	local item = queue[qi++];
	local t = item[0], path = item[1];
	local lines = [];
	local objkeys = false;
	if (typeof t == "table")
	{
		local entries = [];
		foreach (k, v in t)
		{
			if (typeof k != "string" && typeof k != "integer") objkeys = true;
			entries.push([(typeof k == "string" ? k : typeof k == "integer" ? "" + k : typeof k) + "=" + desc(v), v]);
		}
		if (objkeys) entries.sort(@(a, b) a[0] <=> b[0]);
		foreach (e in entries)
		{
			lines.push(e[0]);
			local v = e[1];
			if ((typeof v == "table" || typeof v == "array") && !(v in seen)) { seen[v] <- true; queue.push([v, path + "." + e[0].slice(0, e[0].find("="))]); }
		}
		local d = t.getdelegate();
		if (d != null && !(d in seen)) { seen[d] <- true; queue.push([d, path + ".<delegate>"]); }
	}
	else
	{
		foreach (i, v in t)
		{
			lines.push(desc(v));
			if ((typeof v == "table" || typeof v == "array") && !(v in seen)) { seen[v] <- true; queue.push([v, path + "[" + i + "]"]); }
		}
	}
	::__dump.push(path + " " + typeof t + " " + lines.len() + ": " + (lines.len() ? lines.reduce(@(a, b) a + " | " + b) : ""));
}
