// Normal line heights of the game's fonts: for each family and whole
// size from 6 to 40px, a one-line block's height (its normal line height) and an inline
// span's (the font's ascent + descent). The original and our host answer differently
// for the same font and size (FreeUniversal-Regular 12px: 16px there, 15px here).
// Results: tests/goldens/font_metrics.json. ES5 only.
BbProbe.run = function ()
{
	var families = ['Arsenal-Regular', 'Arsenal-Italic', 'FreeUniversal-Regular', 'FreeUniversal-Italic',
		'Cinzel-Regular', 'Cinzel-Bold', 'Times New Roman'];
	var host = document.createElement('div');
	host.setAttribute('style', 'position: absolute; left: 0; top: 0; visibility: hidden;');
	document.body.appendChild(host);
	// Let the fonts load before measuring.
	setTimeout(function ()
	{
		var out = {};
		for (var f = 0; f < families.length; f++)
		{
			var rows = [];
			for (var size = 6; size <= 40; size++)
			{
				var block = document.createElement('div');
				block.setAttribute('style', 'font-family: \'' + families[f] + '\'; font-size: ' + size + 'px; line-height: normal; white-space: nowrap;');
				var span = document.createElement('span');
				span.textContent = 'Hxgjq';
				block.appendChild(span);
				host.appendChild(block);
				rows.push([size, block.getBoundingClientRect().height, span.getBoundingClientRect().height]);
				host.removeChild(block);
			}
			out[families[f]] = rows;
		}
		BbProbe.send('font_metrics', JSON.stringify(out));
		BbProbe.done();
	}, 3000);
};
