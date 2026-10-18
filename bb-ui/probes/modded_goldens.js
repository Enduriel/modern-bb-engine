// UI parity goldens with the install's mods: the original's main menu once Modern
// Hooks has loaded every mod's JS (MSU's "Mod Options" button shows), and MSU's mod
// settings screen, left by its Cancel (which saves nothing). Taken with BBSnapshot
// (bb-ui/src/parity/snapshot.js, prepended when running it; see README.md), in menu
// mode with --keep-mods. ES5 only.
function bbModdedButton(label)
{
	var all = document.querySelectorAll('.ui-control');
	for (var i = 0; i < all.length; i++)
	{
		if (all[i].offsetParent !== null && all[i].textContent.replace(/^\s+|\s+$/g, '') === label)
		{
			return all[i];
		}
	}
	return null;
}

BbProbe.run = function ()
{
	var waited = 0;
	var poll = function ()
	{
		if (bbModdedButton('Mod Options') === null)
		{
			waited += 500;
			if (waited > 180000)
			{
				console.error('PROBE_FAIL (js) no Mod Options button after 180 s');
				BbProbe.done();
				return;
			}
			setTimeout(poll, 500);
			return;
		}
		setTimeout(function ()
		{
			BbProbe.send('golden_modded_main_menu', BBSnapshot.take());
			$(bbModdedButton('Mod Options')).click();
			setTimeout(function ()
			{
				BbProbe.send('golden_modded_mod_options', BBSnapshot.take());
				var cancel = bbModdedButton('Cancel');
				if (cancel !== null)
				{
					$(cancel).click();
				}
				setTimeout(function () { BbProbe.done(); }, 2000);
			}, 3000);
		}, 3000);
	};
	poll();
};
