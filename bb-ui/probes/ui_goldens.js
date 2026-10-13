// UI parity goldens: snapshots of the original's main menu, its
// new-campaign page and its Options screen, taken with BBSnapshot
// (bb-ui/src/parity/snapshot.js, prepended to this file when running it; see README.md).
// ES5 only.
function bbButton(label)
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

// What a player's click on the button runs (the buttons' handlers are jQuery click
// handlers; the scripts they call run inline).
function bbClick(label)
{
	var button = bbButton(label);
	if (button === null)
	{
		console.error('PROBE js: no visible button ' + label);
		return;
	}
	$(button).click();
}

BbProbe.run = function ()
{
	var steps = [
		function () { BbProbe.send('golden_main_menu', BBSnapshot.take()); bbClick('New Campaign'); },
		function () { BbProbe.send('golden_new_campaign', BBSnapshot.take()); bbClick('Cancel'); },
		function () { bbClick('Options'); },
		function () { BbProbe.send('golden_options', BBSnapshot.take()); bbClick('Cancel'); },
		function () { BbProbe.done(); }
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
			// Let menus finish sliding in.
			setTimeout(step, 3000);
		}
	};
	setTimeout(step, 3000);
};
