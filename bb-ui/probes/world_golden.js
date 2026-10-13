// UI parity goldens of the world: the original right after a new campaign,
// as the probe harness's campaign mode starts it ("A New Company", Veteran settings,
// `--seed` as the map seed). Snapshots, each 3 s after its step:
//   golden_world_intro      the origin's intro event;
//   golden_world            after its "Huzzah!": the world screen;
//   golden_world_character  the character screen (the top bar's Brothers button);
//   golden_world_town       after closing it, the nearest town's screen.
// The steps between run through world_golden.nut (`probeStep`). Taken with BBSnapshot
// (bb-ui/src/parity/snapshot.js, prepended to this file when running it; see
// README.md). ES5 only.
function bbShowsText(text)
{
	var walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT, null, false);
	for (var node = walker.nextNode(); node !== null; node = walker.nextNode())
	{
		if (node.nodeValue.indexOf(text) >= 0 && node.parentNode.offsetParent !== null)
		{
			return true;
		}
	}
	return false;
}

function bbClickButton(label)
{
	var all = document.querySelectorAll('.ui-control');
	for (var i = 0; i < all.length; i++)
	{
		if (all[i].offsetParent !== null && all[i].textContent.replace(/^\s+|\s+$/g, '') === label)
		{
			$(all[i]).click();
			return;
		}
	}
	console.error('PROBE js: no visible button ' + label);
}

function bbStep(step)
{
	SQ.call(Screens.WorldScreen.mSQHandle, 'probeStep', step);
}

BbProbe.run = function ()
{
	var steps = [
		function () { BbProbe.send('golden_world_intro', BBSnapshot.take()); bbClickButton('Huzzah!'); },
		function () { BbProbe.send('golden_world', BBSnapshot.take()); bbStep('character'); },
		function () { BbProbe.send('golden_world_character', BBSnapshot.take()); bbStep('closeCharacter'); },
		function () { bbStep('town'); },
		function () { BbProbe.send('golden_world_town', BBSnapshot.take()); BbProbe.done(); }
	];
	var next = 0;
	var step = function ()
	{
		try
		{
			steps[next++]();
		}
		catch (e)
		{
			console.error('PROBE_FAIL (js) ' + e);
			BbProbe.done();
			return;
		}
		if (next < steps.length)
		{
			setTimeout(step, 3000);
		}
	};
	var waited = 0;
	var poll = function ()
	{
		if (bbShowsText('Huzzah!'))
		{
			// As long as our side waits: the event slides in.
			setTimeout(step, 3000);
			return;
		}
		waited += 500;
		if (waited > 120000)
		{
			console.error('PROBE_FAIL (js) no intro event after 120 s');
			BbProbe.done();
			return;
		}
		setTimeout(poll, 500);
	};
	poll();
};
