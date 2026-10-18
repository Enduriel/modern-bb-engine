// Which fonts the generic and system families are: with MSU, vanilla's
// `.default-font-*` text is `monospace, Arimo`, and Modern Hooks' popup is `monospace`.
// The width of one sample in each family, measured once the page has had the
// chance to load them (ES5 only). The original's answers are
// `tests/goldens/font_families.json`; `ui_parity` runs this file in our page.
BbProbe.run = function ()
{
	var families = ['monospace', 'serif', 'sans-serif', 'Courier New', 'Arial', 'Times New Roman', 'Arimo',
		'monospace, Arimo', 'NoSuchFontAtAll'];
	var sample = 'Mmi0 The quick brown fox, 1.5!';
	var host = document.createElement('div');
	host.setAttribute('style', 'position: absolute; left: 0; top: 0; visibility: hidden;');
	document.body.appendChild(host);
	var spans = [];
	for (var i = 0; i < families.length; i++)
	{
		var block = document.createElement('div');
		var span = document.createElement('span');
		span.setAttribute('style', 'white-space: nowrap; font-size: 16px; font-family: ' + families[i] + ';');
		span.textContent = sample;
		block.appendChild(span);
		host.appendChild(block);
		spans.push([families[i], span, block]);
	}
	(BbProbe.later || setTimeout)(function ()
	{
		var out = {};
		for (var j = 0; j < spans.length; j++)
		{
			var r = spans[j][1].getBoundingClientRect();
			// The width tells the font; heights round differently anyway.
			out[spans[j][0]] = Math.round(r.width);
		}
		// Monospace fonts share widths (0.6em a glyph); the line tells them apart (Courier
		// New 18px at 16px, DejaVu Sans Mono 19).
		out['monospace line'] = Math.round(spans[0][2].getBoundingClientRect().height);
		document.body.removeChild(host);
		BbProbe.send('families', JSON.stringify(out));
		BbProbe.done();
	}, 3000);
};
