// mode: menu
// Math.rand / seedRandom / seedRandomString sequences.
local function p( _k, _f )
{
	local v;
	try { v = _f(); } catch (e) { v = "ERROR " + e; }
	::logInfo("PROBE " + _k + " = " + typeof v + " " + v);
}
local function seq( _n, _f )
{
	local s = "";
	for (local i = 0; i < _n; i++) s += (i > 0 ? "," : "") + _f();
	return s;
}
::Math.seedRandom(42);
p("seed42 rand(0,1000)", @() seq(12, @() ::Math.rand(0, 1000)));
p("seed42 then rand(-5,5)", @() seq(12, @() ::Math.rand(-5, 5)));
p("rand(5,5)", @() ::Math.rand(5, 5));
p("rand(10,1)", @() ::Math.rand(10, 1));
p("rand(0.5,3.7)", @() seq(6, @() ::Math.rand(0.5, 3.7)));
::Math.seedRandom(0);
p("seed0 rand(1,100)", @() seq(8, @() ::Math.rand(1, 100)));
::Math.seedRandom(-7);
p("seed-7 rand(1,100)", @() seq(8, @() ::Math.rand(1, 100)));
::Math.seedRandom(4294967296 + 5);
p("seed2^32+5 rand(1,100)", @() seq(8, @() ::Math.rand(1, 100)));
::Math.seedRandomString("rust-engine-test");
p("seedString rand(0,99)", @() seq(12, @() ::Math.rand(0, 99)));
::Math.seedRandomString("");
p("seedString empty rand(0,99)", @() seq(8, @() ::Math.rand(0, 99)));
