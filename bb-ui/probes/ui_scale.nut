// The UI scale probe: what the UI scale setting does to the page. 8 s in, this sets the
// video mode's UIScale to `scale` the way the options menu does (::Settings.setVideoMode);
// the JS half (ui_scale.js) measures the page before and after. The probe runner backs up
// the player's config.cfg and restores it after the run. Menu mode, `--js ui_scale.js`
// (with src/parity/snapshot.js prepended).
// The scale to set; ui_scale.js's SCALE_NAME names the golden after it (1.5: "ui150").
local scale = 1.5;
::Time.scheduleEvent(::TimeUnit.Real, 8000, function ( _data )
{
	local mode = ::Settings.getVideoMode();
	::logInfo("PROBE uiscale before: " + mode.Width + "x" + mode.Height + " UIScale=" + mode.UIScale + " SceneScale=" + mode.SceneScale);
	mode.UIScale = scale;
	::Settings.setVideoMode(mode);
	local now = ::Settings.getVideoMode();
	::logInfo("PROBE uiscale set: UIScale=" + now.UIScale);
}, null);
::logInfo("PROBE sq: ui scale scheduled");
