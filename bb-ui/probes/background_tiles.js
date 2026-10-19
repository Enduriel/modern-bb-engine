// How Coherent paints a box's background image when it tiles: a
// screenshot of the main menu (menu mode, `--shots DIR --size 2560x1440`), then a card
// of test boxes over the whole screen, and their rects. Each box has a blue background
// colour under its image, so its edges show; the boxes are styled from a style sheet, as
// the game's are. `BBTileCard.show()` draws the same card in ours (`tests/ui_parity.rs`,
// or `boot_menu_ui --click "js:…" --after 2`). ES5 only.
//
// The main menu's buttons (`button_04_default.png`, 230x43 in a 46px box) draw every
// tile upside down in the original, but not where the tile covers the box or the image
// doesn't repeat. The other boxes (an asymmetric 64x64 picture, mostly at 40x30) find
// where that starts and ends. Which boxes the original draws upside down is
// `tests/goldens/background_tiles.json`, read off the card's screenshot (each box
// against its picture as it is and upside down).
var BBTileCard = {
	button: 'coui://gfx/ui/skin/button_04_default.png',
	picture: 'coui://gfx/ui/images/dlc_4.png',
	// [name, image, box width, box height, background-size, background-repeat, extra style]
	rows: [
		['menu', 'button', 230, 46, '23.0rem 4.3rem', 'repeat', ''],
		['norepeat', 'button', 230, 46, '23.0rem 4.3rem', 'no-repeat', ''],
		['tall', 'button', 230, 60, '23.0rem 4.3rem', 'repeat', ''],
		['exact', 'button', 230, 43, '23.0rem 4.3rem', 'repeat', ''],
		['natural', 'button', 230, 46, 'auto', 'repeat', ''],
		['double', 'button', 230, 90, '230px 86px', 'repeat', ''],
		['pos', 'button', 230, 46, '23.0rem 4.3rem', 'repeat', 'background-position: 0px 10px;'],
		['bottom', 'button', 230, 46, '23.0rem 4.3rem', 'repeat', 'background-position: 0% 100%;'],
		['padded', 'button', 230, 46, '23.0rem 4.3rem', 'repeat', 'padding-top: 6px; height: 40px;'],
		['bordered', 'button', 230, 46, '23.0rem 4.3rem', 'repeat', 'border-top: 6px solid rgb(0, 255, 0); height: 40px;'],
		// Second column: the picture at 40x30.
		['p-exact', 'picture', 40, 30, '40px 30px', 'repeat', ''],
		['p-taller', 'picture', 40, 31, '40px 30px', 'repeat', ''],
		['p-wider', 'picture', 41, 30, '40px 30px', 'repeat', ''],
		['p-across', 'picture', 120, 30, '40px 30px', 'repeat', ''],
		['p-down', 'picture', 40, 90, '40px 30px', 'repeat', ''],
		['p-both', 'picture', 120, 90, '40px 30px', 'repeat', ''],
		['p-repeat-x', 'picture', 120, 90, '40px 30px', 'repeat-x', ''],
		['p-repeat-y', 'picture', 120, 90, '40px 30px', 'repeat-y', ''],
		['p-offset', 'picture', 40, 30, '40px 30px', 'repeat', 'background-position: 0px 10px;'],
		['p-smaller', 'picture', 30, 20, '40px 30px', 'repeat', ''],
		['p-stretched', 'picture', 120, 90, '100% 100%', 'repeat', ''],
		['p-natural', 'picture', 120, 90, 'auto', 'repeat', ''],
		['p-norepeat', 'picture', 120, 90, '40px 30px', 'no-repeat', ''],
		['p-cover', 'picture', 120, 90, 'cover', 'repeat', ''],
		['p-centred', 'picture', 120, 90, '40px 30px', 'repeat', 'background-position: 50% 50%;'],
		['p-padded', 'picture', 40, 30, '40px 30px', 'repeat', 'padding: 5px;'],
		['p-auto-height', 'picture', 120, 90, '40px auto', 'repeat', ''],
		['q-narrow-repeat-y', 'picture', 44, 90, '40px 120px', 'repeat-y', ''],
		['q-short-repeat-x', 'picture', 120, 34, '160px 30px', 'repeat-x', ''],
		['q-norepeat-offset', 'picture', 40, 30, '40px 30px', 'no-repeat', 'background-position: 0px 10px;'],
		['q-negative', 'picture', 40, 30, '40px 30px', 'repeat', 'background-position: -10px 0px;'],
		['q-overhang', 'picture', 40, 30, '60px 45px', 'repeat', 'background-position: -5px -5px;'],
		['q-border-clip', 'picture', 40, 30, '40px 30px', 'repeat', 'border: 4px solid rgb(0, 255, 0);'],
		['q-padding-clip', 'picture', 40, 30, '40px 30px', 'repeat', 'border: 4px solid rgb(0, 255, 0); background-clip: padding-box;'],
		['q-contain', 'picture', 120, 90, 'contain', 'repeat', ''],
		['q-percent', 'picture', 120, 90, '50% 50%', 'repeat', '']
	],
	show: function ()
	{
		var card = document.createElement('div');
		card.id = 'bb-tile-card';
		card.setAttribute('style', 'position: fixed; left: 0; top: 0; width: 2560px; height: 1440px; z-index: 99999; background: rgb(0, 0, 0);');
		var css = '';
		var top = 100, column = 0;
		for (var i = 0; i < BBTileCard.rows.length; i++)
		{
			var r = BBTileCard.rows[i];
			if (i === 10)
			{
				top = 100;
				column = 1;
			}
			css += '.bb-tile-' + r[0] + ' { position: absolute; left: ' + [100, 900, 1500][column] + 'px; top: ' + top + 'px; width: ' + r[2] +
				'px; height: ' + r[3] + 'px; background-color: rgb(0, 0, 255); background-image: url("' + BBTileCard[r[1]] +
				'"); background-size: ' + r[4] + '; background-repeat: ' + r[5] + '; ' + r[6] + ' }\n';
			var box = document.createElement('div');
			box.className = 'bb-tile-' + r[0];
			card.appendChild(box);
			top += column === 0 ? 110 : 100;
			if (column === 1 && top > 1300)
			{
				top = 100;
				column = 2;
			}
		}
		var sheet = document.createElement('style');
		sheet.id = 'bb-tile-card-style';
		sheet.textContent = css;
		document.head.appendChild(sheet);
		// The menu's own button (button-4 in an l-center-like 230x46 box), and plain text
		// set like its label.
		var holder = $('<div style="position: absolute; left: 500px; top: 100px; width: 230px; height: 46px; line-height: 3.0rem;"></div>');
		$(card).append(holder);
		holder.createTextButton('New Campaign', null, 'bb-tile-button', 4);
		var text = document.createElement('div');
		text.className = 'bb-tile-text';
		text.setAttribute('style', 'position: absolute; left: 500px; top: 210px; width: 230px; height: 46px; line-height: 30px; font-family: FreeUniversal-Regular; font-size: 18px; color: rgb(255, 255, 255); background-color: rgb(0, 0, 255); text-align: center;');
		text.textContent = 'New Campaign';
		card.appendChild(text);
		document.body.appendChild(card);
	},
	rects: function ()
	{
		var out = {};
		var all = document.querySelectorAll('#bb-tile-card *');
		for (var i = 0; i < all.length; i++)
		{
			var r = all[i].getBoundingClientRect();
			out[i + ' ' + all[i].className] = [r.left, r.top, r.width, r.height];
		}
		var menu = document.querySelectorAll('.main-menu-module .ui-control');
		for (var j = 0; j < menu.length; j++)
		{
			var m = menu[j].getBoundingClientRect();
			var label = menu[j].querySelector('.label');
			var l = label === null ? null : label.getBoundingClientRect();
			out['menu ' + menu[j].textContent] = [m.left, m.top, m.width, m.height, l === null ? null : l.top, l === null ? null : l.height,
				window.getComputedStyle(menu[j]).backgroundPosition, window.getComputedStyle(menu[j]).backgroundSize];
		}
		out.viewport = [window.innerWidth, window.innerHeight];
		return JSON.stringify(out);
	}
};

BbProbe.run = function ()
{
	setTimeout(function ()
	{
		BbProbe.shot('menu', function (error)
		{
			if (error !== null) console.error('PROBE js: menu shot failed: ' + error);
			BBTileCard.show();
			setTimeout(function ()
			{
				BbProbe.send('rects', BBTileCard.rects());
				BbProbe.shot('card', function (error2)
				{
					if (error2 !== null) console.error('PROBE js: card shot failed: ' + error2);
					BbProbe.done();
				});
			}, 3000);
		});
	}, 4000);
};
