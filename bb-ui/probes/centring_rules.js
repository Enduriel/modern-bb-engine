// Old-WebKit centring by `auto` margins. ES5 only. An
// absolutely positioned box between its insets with `margin: auto` (vanilla has ~270 such
// rules vertically, ~150 horizontally: button labels, icons): where does it go when the
// room left is odd, and when there is less room than the box? Old WebKit lays out in
// whole pixels; Chromium splits the room in halves.
//
// At UI scales 1, 1.25 and 1.5 (`centring_rules.nut` sets them, 6 s apart), in screen
// pixels: the box's offset in CSS pixels times the scale, rounded as painting snaps it.
// Declared in a style sheet, as vanilla does, and measured a moment later (our host
// rewrites style sheets as they arrive). The original's answers are
// `tests/goldens/centring_rules.json`; the `ui_parity` test sets each scale in our page
// and calls `BbProbe.measureCentring`.
BbProbe.measureCentring = function (scale, key, done)
{
	// Each: [name, axis, parent size, start inset, end inset, box size].
	var cases = [['even', 'y', 100, 0, 0, 91], ['odd', 'y', 101, 0, 0, 91],
		['negativeHalf', 'y', 80, 0, 0, 91], ['negativeWhole', 'y', 81, 0, 0, 91],
		['insetHalf', 'y', 100, 3, 0, 90], ['insetNegativeHalf', 'y', 50, 30, 0, 27],
		['horizontalHalf', 'x', 100, 0, 0, 91], ['horizontalWhole', 'x', 101, 0, 0, 91],
		['horizontalInsetHalf', 'x', 100, 3, 0, 90], ['horizontalNegativeHalf', 'x', 80, 0, 0, 91]];
	var css = '';
	for (var i = 0; i < cases.length; i++)
	{
		var k = cases[i];
		var y = k[1] === 'y';
		css += '.probe-centring-' + k[0] + ' { position: absolute; left: 0; top: 0; width: ' + (y ? 50 : k[2]) +
			'px; height: ' + (y ? k[2] : 50) + 'px; }\n';
		css += '.probe-centring-' + k[0] + ' > div { position: absolute; margin: auto; ' + (y
			? 'left: 0; width: 10px; top: ' + k[3] + 'px; bottom: ' + k[4] + 'px; height: ' + k[5] + 'px; }\n'
			: 'top: 0; height: 10px; left: ' + k[3] + 'px; right: ' + k[4] + 'px; width: ' + k[5] + 'px; }\n');
	}
	var sheet = document.createElement('style');
	sheet.textContent = css;
	document.head.appendChild(sheet);
	var parents = [];
	for (var j = 0; j < cases.length; j++)
	{
		var parent = document.createElement('div');
		parent.className = 'probe-centring-' + cases[j][0];
		parent.appendChild(document.createElement('div'));
		document.body.appendChild(parent);
		parents.push(parent);
	}
	setTimeout(function ()
	{
		var centring = {};
		for (var c = 0; c < cases.length; c++)
		{
			var y = cases[c][1] === 'y';
			var parent = parents[c];
			var at = parent.firstChild.getBoundingClientRect(), from = parent.getBoundingClientRect();
			centring[cases[c][0]] = String(Math.round((y ? at.top - from.top : at.left - from.left) * scale));
			document.body.removeChild(parent);
		}
		document.head.removeChild(sheet);
		BbProbe.send(key, JSON.stringify(centring));
		done();
	}, 1000);
};

// The original's run: scale 1 now, then 1.25 and 1.5 once the scripts have set them.
BbProbe.run = function ()
{
	BbProbe.measureCentring(1, 'centring', function ()
	{
		setTimeout(function ()
		{
			BbProbe.measureCentring(1.25, 'centring125', function ()
			{
				setTimeout(function ()
				{
					BbProbe.measureCentring(1.5, 'centring150', function () { BbProbe.done(); });
				}, 5000);
			});
		}, 6000);
	});
};
