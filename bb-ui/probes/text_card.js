// A test card for how the page's text is drawn (weight, antialiasing, shadow): the
// game's fonts, sizes and colours on a flat dark panel, over the whole screen, with and
// without the game's 1px shadow. `BbProbe.shot('card')` in the original (menu mode,
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
		['Cinzel-Regular', 24, 'rgb(215, 177, 116)', false]
	],
	sample: 'New Campaign Ulf the Merciless 2,400 quick brown fox',
	show: function ()
	{
		var card = document.createElement('div');
		card.id = 'bb-text-card';
		card.setAttribute('style', 'position: fixed; left: 0; top: 0; width: 2560px; height: 1440px; z-index: 99999; background: rgb(40, 30, 24);');
		for (var i = 0; i < BBTextCard.rows.length; i++)
		{
			var r = BBTextCard.rows[i];
			var line = document.createElement('div');
			line.setAttribute('style', 'position: absolute; left: 100px; top: ' + (100 + i * 80) + 'px; white-space: nowrap; font-family: ' + r[0] +
				'; font-size: ' + r[1] + 'px; color: ' + r[2] + ';' + (r[3] ? ' text-shadow: 1px 1px 1px rgb(0, 0, 0);' : ''));
			line.textContent = BBTextCard.sample;
			card.appendChild(line);
		}
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
