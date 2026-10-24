// The turn bar's first-slot race (turnbar_race.js): call the page once a frame.
local ui = ::BbProbe.UI;
::TurnbarRaceFrame <- 0;
::TurnbarRaceTick <- function( _d )
{
	::TurnbarRaceFrame++;
	::BbProbe.UI.JSHandle.asyncCall("tbTick", ::TurnbarRaceFrame);
	if (::TurnbarRaceFrame < 30000)
	{
		::Time.scheduleEvent(::TimeUnit.Real, 1, ::TurnbarRaceTick, null);
	}
};
ui.tbStart <- function()
{
	::logInfo("PROBE turnbar race: started");
	::Time.scheduleEvent(::TimeUnit.Real, 1, ::TurnbarRaceTick, null);
};
