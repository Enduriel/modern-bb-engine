// The mouse events the original's page gets along modded_goldens.js's flow, with no input
// sent (menu mode, --keep-mods): where the probe's cursor is, and when Dev Console's
// element inspector box (which hides on any body mouseover) appears and hides. Found
// (2026-10-20): the page gets a mousemove every frame at (640, 360) or (640, 380), the
// headless display's pointer; mouseovers come as the menu appears under it; the box
// comes after them (late JS) and shows; MSU's settings screen opening under the cursor
// brings a mouseover, which hides it. ES5 only.
var bbMouseLog = [];
var bbMouseT0 = Date.now();

function bbInspector()
{
	var el = document.querySelector('.dom-element-inspector');
	if (el === null)
	{
		return 'absent';
	}
	return window.getComputedStyle(el).display + ' state=' + (typeof ElementInspector !== 'undefined' ? ElementInspector.State : 'n/a');
}

(function ()
{
	var types = ['mouseover', 'mouseout', 'mousemove', 'mouseenter', 'mouseleave'];
	for (var i = 0; i < types.length; i++)
	{
		document.addEventListener(types[i], function (ev)
		{
			if (bbMouseLog.length >= 400)
			{
				return;
			}
			var t = ev.target;
			var entry = [Date.now() - bbMouseT0, ev.type, ev.clientX, ev.clientY, t && t.tagName ? t.tagName + '.' + String(t.className).slice(0, 50) : String(t)];
			if (ev.type === 'mouseover')
			{
				entry.push('before:' + bbInspector());
				setTimeout(function () { entry.push('after:' + bbInspector()); }, 0);
			}
			bbMouseLog.push(entry);
		}, true);
	}
})();

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

var bbSeen = {};
var bbWatch = setInterval(function ()
{
	var now = Date.now() - bbMouseT0;
	var names = { inspector: '.dom-element-inspector', menu: '.main-menu-screen' };
	for (var k in names)
	{
		if (!(k in bbSeen) && document.querySelector(names[k]) !== null)
		{
			bbSeen[k] = now;
			bbMouseLog.push([now, 'appeared ' + k, bbInspector()]);
		}
	}
}, 5);

BbProbe.run = function ()
{
	bbMouseLog.push([Date.now() - bbMouseT0, 'run', bbInspector()]);
	var waited = 0;
	var poll = function ()
	{
		if (bbButton('Mod Options') === null && waited < 180000)
		{
			waited += 500;
			setTimeout(poll, 500);
			return;
		}
		setTimeout(function ()
		{
			var centre = document.elementFromPoint(1280, 720);
			BbProbe.send('menu', JSON.stringify({ inspector: bbInspector(), events: bbMouseLog.length, at_centre: centre ? centre.className : null }));
			bbMouseLog.push(['--- click Mod Options']);
			$(bbButton('Mod Options')).click();
			setTimeout(function ()
			{
				clearInterval(bbWatch);
				BbProbe.send('options', JSON.stringify({ inspector: bbInspector() }));
				BbProbe.send('events', JSON.stringify(bbMouseLog));
				var cancel = bbButton('Cancel');
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
