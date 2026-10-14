// The UI scale probe: what the UI scale setting does to the page. 8 s in, this sets the
// video mode's UIScale to 1.5 the way the options menu does (::Settings.setVideoMode);
// the JS half (ui_scale.js) measures the page before and after. The probe runner backs up
// the player's config.cfg and restores it after the run. Menu mode, `--js ui_scale.js`
// (with src/parity/snapshot.js prepended).
::Time.scheduleEvent(::TimeUnit.Real, 8000, function ( _data )
{
	local mode = ::Settings.getVideoMode();
	::logInfo("PROBE uiscale before: " + mode.Width + "x" + mode.Height + " UIScale=" + mode.UIScale + " SceneScale=" + mode.SceneScale);
	mode.UIScale = 1.5;
	::Settings.setVideoMode(mode);
	local now = ::Settings.getVideoMode();
	::logInfo("PROBE uiscale set: UIScale=" + now.UIScale);
}, null);
::logInfo("PROBE sq: ui scale scheduled");
