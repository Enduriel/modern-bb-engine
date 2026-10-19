// A test card for how the page's text is drawn (weight, antialiasing, shadow): the
// game's fonts, sizes and colours on a flat dark panel, over the whole screen, with and
// without the game's text shadows. `BbProbe.shot('card')` in the original (menu mode,
// `--shots DIR`, 2560x1440); ours draws the same card for `boot_menu_ui --out`. Rows at
// fixed places (100px from the left, 100px + 80px per row from the top). ES5 only.
var BBTextCard = {
	rows: [
		['FreeUniversal-Regular', 16, 'rgb(215, 177, 116)', false],
		['FreeUniversal-Regular', 16, 'rgb(215, 177, 116)', true],
		['FreeUniversal-Regular', 20, 'rgb(221, 162, 31)', false],
		['FreeUniversal-Regular', 20, 'rgb(221, 162, 31)', true],
		['FreeUniversal-Regular', 13, 'rgb(255, 255, 255)', false],
		['Cinzel-Bold', 20, 'rgb(221, 162, 31)', false],
		['Cinzel-Bold', 28, 'rgb(221, 162, 31)', true],
		['Cinzel-Regular', 24, 'rgb(215, 177, 116)', false],
		// Vanilla's other shadows (rows 8-11).
		['FreeUniversal-Regular', 20, 'rgb(221, 162, 31)', '2px 2px 1px #000'],
		['Cinzel-Bold', 28, 'rgb(221, 162, 31)', '1px 1px #A36426, -1px -1px #000'],
		['FreeUniversal-Regular', 16, 'rgb(215, 177, 116)', '1px 1px 1px rgba(40, 40, 40, 0.8)'],
		['FreeUniversal-Regular', 20, 'rgb(221, 162, 31)', '1px 1px #000']
	],
	sample: 'New Campaign Ulf the Merciless 2,400 quick brown fox',
	show: function ()
	{
		// Shadows in a style sheet, where vanilla declares them (and our compat.js rewrites
		// blurred ones).
		var sheet = document.createElement('style');
		var css = '';
		var card = document.createElement('div');
		card.id = 'bb-text-card';
		card.setAttribute('style', 'position: fixed; left: 0; top: 0; width: 2560px; height: 1440px; z-index: 99999; background: rgb(40, 30, 24);');
		for (var i = 0; i < BBTextCard.rows.length; i++)
		{
			var r = BBTextCard.rows[i];
			var line = document.createElement('div');
			line.className = 'bb-text-card-' + i;
			line.setAttribute('style', 'position: absolute; left: 100px; top: ' + (100 + i * 80) + 'px; white-space: nowrap; font-family: ' + r[0] +
				'; font-size: ' + r[1] + 'px; color: ' + r[2] + ';');
			if (r[3] !== false)
			{
				css += '.bb-text-card-' + i + ' { text-shadow: ' + (r[3] === true ? '1px 1px 1px #000' : r[3]) + '; }\n';
			}
			line.textContent = BBTextCard.sample;
			card.appendChild(line);
		}
		sheet.textContent = css;
		document.head.appendChild(sheet);
		document.body.appendChild(card);
	}
};

BbProbe.run = function ()
{
	BBTextCard.show();
	setTimeout(function ()
	{
		BbProbe.shot('card', function (error)
		{
			if (error !== null)
			{
				console.error('PROBE js: shot failed: ' + error);
			}
			BbProbe.done();
		});
	}, 4000);
};
