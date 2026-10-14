// mode: menu
// Math natives: return types and edge values.
local function p( _k, _f )
{
	local v;
	try { v = _f(); } catch (e) { v = "ERROR " + e; }
	::logInfo("PROBE " + _k + " = " + typeof v + " " + v);
}
foreach (x in [2.5, -2.5, 2.4, -2.6, 3, -3, 0.5, -0.5, 1e9 + 0.5])
{
	p("floor(" + x + ")", @() ::Math.floor(x));
	p("ceil(" + x + ")", @() ::Math.ceil(x));
	p("round(" + x + ")", @() ::Math.round(x));
}
p("abs(-3)", @() ::Math.abs(-3));
p("abs(-3.5)", @() ::Math.abs(-3.5));
p("absf(-3)", @() ::Math.absf(-3));
p("min(2, 7)", @() ::Math.min(2, 7));
p("min(2.5, 7)", @() ::Math.min(2.5, 7));
p("max(-1, -9)", @() ::Math.max(-1, -9));
p("minf(2.5, 7)", @() ::Math.minf(2.5, 7));
p("maxf(2, 7)", @() ::Math.maxf(2, 7));
p("pow(2, 10)", @() ::Math.pow(2, 10));
p("pow(2.5, 0.5)", @() ::Math.pow(2.5, 0.5));
p("pow(-8, 0.333)", @() ::Math.pow(-8, 0.333));
p("getAngleTo east", @() ::Math.getAngleTo(::createVec(0, 0), ::createVec(10, 0)));
p("getAngleTo north", @() ::Math.getAngleTo(::createVec(0, 0), ::createVec(0, 10)));
p("getAngleTo sw", @() ::Math.getAngleTo(::createVec(5, 5), ::createVec(-5, -5)));
p("round(0.49999997)", @() ::Math.round(0.49999997));
p("round(-0.49999997)", @() ::Math.round(-0.49999997));
p("abs(-2147483648)", @() ::Math.abs(-2147483647 - 1));
p("getAngleTo ne", @() ::Math.getAngleTo(::createVec(0, 0), ::createVec(3, 3)));
p("getAngleTo w", @() ::Math.getAngleTo(::createVec(0, 0), ::createVec(-7, 0)));
p("getAngleTo frac", @() ::Math.getAngleTo(::createVec(1.5, 2.25), ::createVec(-3.75, 8.5)));
