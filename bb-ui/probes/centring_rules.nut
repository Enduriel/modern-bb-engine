// The SQ half of centring_rules.js: the UI scale set to 1.25 at 4 s and 1.5 at 10 s, as
// the Options screen sets it, while the page measures at 1, then each (menu mode; the
// runner restores config.cfg).
foreach (i, scale in [1.25, 1.5])
{
	::Time.scheduleEvent(::TimeUnit.Real, 4000 + 6000 * i, function ( _scale )
	{
		local mode = ::Settings.getVideoMode();
		mode.UIScale = _scale;
		::Settings.setVideoMode(mode);
		::logInfo("PROBE uiscale " + ::Settings.getVideoMode().UIScale);
	}, scale);
}
