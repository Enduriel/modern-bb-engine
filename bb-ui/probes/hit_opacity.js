// Hit testing through invisible boxes. With Modern Hooks installed, its error
// popup lies over the main menu's middle at `opacity: 0` (vanilla's `.opacity-none`,
// whose `pointer-events: none` is commented out), yet players click the buttons under
// it. Which element does a point hit: under an `opacity: 0` box, under one at 0.01, and
// at the centres of the main menu's buttons (menu mode, with --keep-mods for the popup).
// ES5 only.
BbProbe.run = function ()
{
	var describe = function (el)
	{
		return el === null ? 'null' : el.tagName.toLowerCase() + (el.id ? '#' + el.id : '') + (el.className ? '.' + String(el.className).replace(/\s+/g, '.') : '');
	};
	var waited = 0;
	var poll = function ()
	{
		var buttons = document.querySelectorAll('.main-menu-module .ui-control.button');
		if (buttons.length === 0 && waited < 180000)
		{
			waited += 500;
			setTimeout(poll, 500);
			return;
		}
		setTimeout(function ()
		{
			var hits = {};
			for (var i = 0; i < buttons.length; i++)
			{
				var r = buttons[i].getBoundingClientRect();
				var label = buttons[i].textContent.replace(/^\s+|\s+$/g, '');
				hits[label] = describe(document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2));
			}
			var cases = [['opacity0', 'opacity: 0'], ['opacity001', 'opacity: 0.01'], ['visibilityHidden', 'visibility: hidden']];
			for (var c = 0; c < cases.length; c++)
			{
				var under = document.createElement('div');
				under.id = 'probe-under-' + cases[c][0];
				under.setAttribute('style', 'position:fixed;left:10px;top:10px;width:50px;height:50px;background:red;z-index:5000');
				var over = document.createElement('div');
				over.id = 'probe-over-' + cases[c][0];
				over.setAttribute('style', 'position:fixed;left:0;top:0;width:100px;height:100px;background:blue;z-index:5001;' + cases[c][1]);
				document.body.appendChild(under);
				document.body.appendChild(over);
				hits[cases[c][0]] = describe(document.elementFromPoint(35, 35));
				document.body.removeChild(under);
				document.body.removeChild(over);
			}
			BbProbe.send('hits', JSON.stringify(hits));
			BbProbe.done();
		}, 3000);
	};
	poll();
};
