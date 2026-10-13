// UI bridge ground truth: value conversions both ways, call/asyncCall return values,
// JS exceptions inside SQ->JS calls, RootScreen's handles. See README.md.

::BBProbeDump <- function( v, depth = 0 )
{
	if (depth > 4) return "...";
	switch (typeof v)
	{
	case "null": return "null";
	case "integer": return "i:" + v;
	case "float": return "f:" + v;
	case "bool": return "b:" + v;
	case "string": return "s:'" + v + "'";
	case "array":
		local s = "[";
		foreach (i, x in v) s += (i > 0 ? "," : "") + ::BBProbeDump(x, depth + 1);
		return s + "](len " + v.len() + ")";
	case "table":
		local s = "{";
		local first = true;
		foreach (k, x in v)
		{
			s += (first ? "" : ",") + ::BBProbeDump(k, depth + 1) + "=" + ::BBProbeDump(x, depth + 1);
			first = false;
		}
		return s + "}";
	default:
		return typeof v;
	}
};

local ui = ::BbProbe.UI;
ui.types <- function( _d ) { ::logInfo("PROBE js2sq " + ::BBProbeDump(_d)); };
ui.pushEchoes <- function()
{
	local h = ::BbProbe.UI.JSHandle;
	// (An array holding a function or instance crashes the original: "Array size mismatch".)
	h.asyncCall("echo", { [1] = "int key", [2.5] = "float key", s = "string key", b = true });
	h.asyncCall("echo", ::Settings.getVideoMode());
	h.asyncCall("echo", function() {});
	h.asyncCall("echo", ::BbProbe.UI.weakref());
	::logInfo("PROBE ret: call -> " + ::BBProbeDump(h.call("retVal", 1)));
	::logInfo("PROBE ret: asyncCall -> " + ::BBProbeDump(h.asyncCall("retVal", 2)));
	try { h.asyncCall("jsThrows", 3); ::logInfo("PROBE throw: asyncCall returned normally"); }
	catch (e) { ::logInfo("PROBE throw: asyncCall raised " + e); }
	h.asyncCall("echo", "after throw");
	try { h.asyncCall("noSuchJsMethod", 4); ::logInfo("PROBE throw: missing method returned normally"); }
	catch (e) { ::logInfo("PROBE throw: missing method raised " + e); }
	try { ::UI.connect("NoSuchScreen", {}); ::logInfo("PROBE connect unknown: returned"); }
	catch (e) { ::logInfo("PROBE connect unknown: raised " + e); }
};
::logInfo("PROBE sq: bridge_values loaded");
