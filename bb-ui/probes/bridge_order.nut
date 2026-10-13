// UI bridge ground truth: are calls between scripts and UI inline? Plus callSQ return
// values, and the receivers the JS side (bridge_order.js) calls. See README.md.
// Ordering is read from log.html, where these rows and the JS console.error rows land in
// execution order.

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
		return s + "]";
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

// ---- JS -> SQ: what arrives, by type and argument count ----
ui.types <- function( _d ) { ::logInfo("PROBE js2sq " + ::BBProbeDump(_d)); };
ui.noArgs <- function() { ::logInfo("PROBE noArgs ran"); };
ui.oneArg <- function( _d ) { ::logInfo("PROBE oneArg ran with " + ::BBProbeDump(_d)); };
ui.twoArgs <- function( _a, _b ) { ::logInfo("PROBE twoArgs ran with " + ::BBProbeDump(_a) + " / " + ::BBProbeDump(_b)); };
ui.logNow <- function() { ::logInfo("PROBE order: sq logNow ran"); };

// ---- callSQ return values ----
ui.ret <- function()
{
	return { i = 1, f = 2.5, w = 3.0, t = 0.1, n = null, a = [null, 3, null], s = "x", b = true, fn = function() {}, vm = ::Settings.getVideoMode(), [7] = "seven" };
};
ui.retInt <- function() { return 42; };
ui.retNull <- function() { return null; };
ui.retNone <- function() {};
ui.throws <- function() { throw "probe error"; };
ui.pM <- function()
{
	::logInfo("PROBE order: sq pM runs, asyncCall pX");
	::BbProbe.UI.JSHandle.asyncCall("pX", null);
	::logInfo("PROBE order: sq pM returns");
	return 5;
};

// ---- SQ -> JS: values ----
ui.pushEchoes <- function()
{
	local h = ::BbProbe.UI.JSHandle;
	h.asyncCall("echo", null);
	h.asyncCall("echo", 5);
	h.asyncCall("echo", 5.0);
	h.asyncCall("echo", 0.1);
	h.asyncCall("echo", [null, 1, null]);
	h.asyncCall("echo", { z = 1, a = 2, m = 3, b = 4, q = 5 });
	h.asyncCall("echo", { fn = function() {}, vm = ::Settings.getVideoMode(), wr = ::BbProbe.UI.weakref(), nested = { x = [1.5, "y"] } });
	h.call("echo", "via call");
	// (A one-argument asyncCall("echoNoArgs") raises "wrong number of parameters".)
};
ui.pCb <- function( _d ) { ::BbProbe.UI.CbRan = true; ::logInfo("PROBE inline: sq pCb ran with " + ::BBProbeDump(_d)); };
ui.CbRan <- false;

// ---- SQ-initiated ordering: are SQ->JS calls run inline? ----
::BbProbe.UI2 <- { function onScreenConnected() { ::logInfo("PROBE inline: sq onScreenConnected (UI2)"); } };
local function inlineTests( _d )
{
	local h = ::BbProbe.UI.JSHandle;
	::logInfo("PROBE inline: start (from a scheduled SQ event)");

	::BbProbe.UI.CbRan = false;
	h.asyncCall("pFn", 1);
	::logInfo("PROBE inline: after asyncCall; sq pCb ran=" + ::BbProbe.UI.CbRan);

	::BbProbe.UI.CbRan = false;
	h.call("pFn", 2);
	::logInfo("PROBE inline: after call; sq pCb ran=" + ::BbProbe.UI.CbRan);

	local h2 = ::UI.connect("BBProbeScreen2", ::BbProbe.UI2);
	::logInfo("PROBE inline: after UI.connect; handle=" + typeof h2);
	h2.asyncCall("mark", "after connect");
	::logInfo("PROBE inline: end");
}
::Time.scheduleEvent(::TimeUnit.Real, 1500, inlineTests, null);

::logInfo("PROBE sq: bridge_order loaded");
