// Modern Hooks' resume in the original (menu mode with --keep-mods): its page
// side calls the scripts' `resumeOnInit` (which returns true) and, given `true` in the
// callback, sets `Hooks.screenInit` and removes its "game is not loading" panel. In the
// original the panel is still there once the menu is up. Does the callback run, and with
// what? ES5 only.
BbProbe.run = function ()
{
	var waited = 0;
	var poll = function ()
	{
		var menu = document.querySelector('.main-menu-module .ui-control');
		if (menu === null && waited < 180000)
		{
			waited += 500;
			setTimeout(poll, 500);
			return;
		}
		setTimeout(function ()
		{
			var answers = {
				hooks: typeof Hooks,
				screenInit: typeof Hooks !== 'undefined' ? String(Hooks.screenInit) : 'n/a',
				errorMsgSet: typeof Hooks !== 'undefined' ? String(Hooks.errorMsg != null) : 'n/a',
				panelInDom: document.getElementById('hooksErrorMessage') !== null
			};
			// The same call again, now, as the page made it.
			if (typeof Hooks !== 'undefined' && typeof SQ !== 'undefined')
			{
				var handle = null;
				for (var key in Screens)
				{
					if (Screens[key] && Screens[key].constructor && Screens[key].constructor.name === 'ModernHooksConnection')
					{
						handle = Screens[key].mSQHandle;
					}
				}
				answers.connectionHandle = handle !== null;
			}
			BbProbe.send('resume', JSON.stringify(answers));
			BbProbe.done();
		}, 3000);
	};
	poll();
};
