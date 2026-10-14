// Old-WebKit text layout. ES5 only.
//  1. Do glyphs advance by whole pixels? A run of one glyph is then that many times
//     one glyph's width, and a line the sum of its glyphs' widths.
//  2. Is text kerned? Kerned, "AV" in Cinzel-Bold (which has a kerning table) is
//     narrower than "A" and "V" apart; also with `text-rendering: optimizeLegibility`
//     or `font-kerning: normal` asked for.
//
// Old WebKit loads a web font once text uses it, laying out with a fallback until
// then: the samples are put in the page first and measured once the fonts are in.
// The original's answers are `tests/goldens/text_rules.json`; the `ui_parity` test runs
// this file in our page (its `BbProbe.later` measures at once) and compares.
BbProbe.run = function ()
{
	var samples = [
		['free16i', 'FreeUniversal-Regular', 16, '', 'i'],
		['free16i10', 'FreeUniversal-Regular', 16, '', 'iiiiiiiiii'],
		['free16e', 'FreeUniversal-Regular', 16, '', 'e'],
		['free16e10', 'FreeUniversal-Regular', 16, '', 'eeeeeeeeee'],
		['free16line', 'FreeUniversal-Regular', 16, '', 'maybe they are greedy for quick spoils of war. Most of them will die on the battlefield. But do'],
		['free13line', 'FreeUniversal-Regular', 13, '', 'not be discouraged. Such is the mercenary life, and the next village will always have new men'],
		['free18line', 'FreeUniversal-Regular', 18, '', 'A large city and trading hub surrounded by barren tundra'],
		['cinzelA', 'Cinzel-Bold', 28, '', 'A'],
		['cinzelV', 'Cinzel-Bold', 28, '', 'V'],
		['cinzelAV', 'Cinzel-Bold', 28, '', 'AV'],
		['cinzelP', 'Cinzel-Bold', 28, '', 'P'],
		['cinzelPA', 'Cinzel-Bold', 28, '', 'PA'],
		['cinzelT', 'Cinzel-Bold', 28, '', 'T'],
		['cinzelo', 'Cinzel-Bold', 28, '', 'o'],
		['cinzelTo', 'Cinzel-Bold', 28, '', 'To'],
		['cinzelAVlegible', 'Cinzel-Bold', 28, 'text-rendering: optimizeLegibility;', 'AV'],
		['cinzelAVkerning', 'Cinzel-Bold', 28, 'font-kerning: normal; -webkit-font-kerning: normal;', 'AV'],
		['cinzelPaused', 'Cinzel-Bold', 28, '', 'PAUSED'],
		['cinzelNew', 'Cinzel-Bold', 20, '', 'New Campaign']
	];
	var host = document.createElement('div');
	host.setAttribute('style', 'position: absolute; left: 0; top: 0; visibility: hidden;');
	document.body.appendChild(host);
	var spans = [];
	for (var i = 0; i < samples.length; i++)
	{
		var block = document.createElement('div');
		var span = document.createElement('span');
		span.setAttribute('style', 'white-space: nowrap; font-family: \'' + samples[i][1] + '\'; font-size: ' +
			samples[i][2] + 'px; ' + samples[i][3]);
		span.textContent = samples[i][4];
		block.appendChild(span);
		host.appendChild(block);
		spans.push(span);
	}
	(BbProbe.later || setTimeout)(function ()
	{
		var widths = {};
		for (var j = 0; j < samples.length; j++)
		{
			widths[samples[j][0]] = spans[j].getBoundingClientRect().width;
		}
		document.body.removeChild(host);
		BbProbe.send('text', JSON.stringify(widths));
		BbProbe.done();
	}, 3000);
};
