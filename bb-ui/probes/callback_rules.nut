// UI bridge ground truth: when does a page call's callback get the scripts'
// answer? Modern Hooks calls `resumeOnInit` (which builds screens, connecting them, and
// returns true) and in the original its callback never sees `true`. Receivers for
// callback_rules.js, each returning true after doing one thing.
local ui = ::BbProbe.UI;
ui.plainRet <- function() { return true; };
ui.asyncRet <- function()
{
	::BbProbe.UI.JSHandle.asyncCall("noop", null);
	return true;
};
::BbProbe.UI3 <- { function onScreenConnected() { ::logInfo("PROBE callbacks: UI3 connected"); } };
ui.connectRet <- function()
{
	::BbProbe.UI.H3 <- ::UI.connect("BBProbeScreen3", ::BbProbe.UI3);
	::logInfo("PROBE callbacks: connected in a call, handle " + typeof ::BbProbe.UI.H3);
	return true;
};
ui.newScreenRet <- function()
{
	::BbProbe.UI.Dialog <- ::new("scripts/ui/screens/dialog_screen");
	return true;
};
