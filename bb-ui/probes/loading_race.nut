// A loading screen shown again while it fades out (menu mode; a lost Start: the main
// menu hides the loading screen as it comes back, and a Start pressed during that fade
// calls show()). Each trial: show, then hide 2 s later, then show `delay` ms after the
// hide (0: in the same script), then 4 s after the hide log what the scripts see. Vanilla's
// show() is the path Start takes (main_menu_state.campaign_menu_module_onStartPressed).
::BbProbe.DeferDone = true;
local screen = ::LoadingScreen;
::ProbeLoadingShown <- 0;
screen.setOnScreenShownListener(function () { ::ProbeLoadingShown++; });
// 1500: after the fade has ended (the control).
local delays = [0, 50, 150, 400, 1500];
foreach (i, delay in delays)
{
	local at = 3000 + i * 8000;
	::Time.scheduleEvent(::TimeUnit.Real, at, function ( _d ) { ::LoadingScreen.show(); }, null);
	::Time.scheduleEvent(::TimeUnit.Real, at + 2000, function ( _d )
	{
		::logInfo("PROBE loading race " + _d + " ms: before the hide visible " + ::LoadingScreen.isVisible());
		::ProbeLoadingShown = 0;
		::LoadingScreen.hide();
		if (_d == 0)
		{
			::LoadingScreen.show();
		}
	}, delay);
	if (delay > 0)
	{
		::Time.scheduleEvent(::TimeUnit.Real, at + 2000 + delay, function ( _d ) { ::LoadingScreen.show(); }, null);
	}
	::Time.scheduleEvent(::TimeUnit.Real, at + 6000, function ( _d )
	{
		::logInfo("PROBE loading race " + _d + " ms: visible " + ::LoadingScreen.isVisible() + " animating " + ::LoadingScreen.isAnimating()
			+ " shown again " + ::ProbeLoadingShown);
		::LoadingScreen.hide();
	}, delay);
}
::Time.scheduleEvent(::TimeUnit.Real, 3000 + delays.len() * 8000 + 1000, function ( _d ) { ::BbProbe.done(); }, null);
