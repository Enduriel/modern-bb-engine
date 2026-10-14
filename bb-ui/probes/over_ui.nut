// What `UI.setCursorPosition`'s answer (is the cursor over the UI?)
// means. Run in menu mode with the JS half, `--js over_ui.js`, which hides the main menu
// and covers the page with eight full-height stripes, 320 page pixels wide each:
//   0 nothing, 1 opaque, 2 opaque with `pointer-events: none`, 3 a transparent element,
//   4 translucent `rgba(0, 0, 0, 0.3)`, 5 opaque at `opacity: 0.01`, 6 opaque at
//   `opacity: 0`, 7 opaque with `visibility: hidden`.
// Earlier runs showed the answer is immediate, true over a full-screen menu or box and
// false over nothing. This asks over each stripe's centre twice: in page pixels
// (x = 160 + 320 i), and at half those coordinates, in case the cursor's space is the
// window's (the probe's window is 1280x720, the page 2560x1440).
::Time.scheduleEvent(::TimeUnit.Real, 10000, function ( _data )
{
	local page = "";
	local half = "";

	for( local i = 0; i < 8; i++ )
	{
		page += i + "=" + ::UI.setCursorPosition(160 + 320 * i, 720) + " ";
		half += i + "=" + ::UI.setCursorPosition(80 + 160 * i, 360) + " ";
	}

	::logInfo("PROBE overui page-pixels " + page);
	::logInfo("PROBE overui half-coordinates " + half);
}, null);
::logInfo("PROBE sq: over_ui scheduled");
