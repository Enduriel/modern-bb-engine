// The UI scale probe, the page half (see ui_scale.nut): the page's geometry before the
// scripts set UIScale 1.5 (at 8 s) and after, plus a snapshot of the main menu after
// (golden_main_menu_ui150). ES5 only; needs BBSnapshot (src/parity/snapshot.js) prepended.
BbProbe.run = function ()
{
	var measure = function ()
	{
		var html = document.documentElement;
		var button = document.querySelector('.main-menu-module .ui-control');
		var r = button ? button.getBoundingClientRect() : null;
		return {
			innerWidth: window.innerWidth,
			innerHeight: window.innerHeight,
			devicePixelRatio: window.devicePixelRatio,
			htmlZoom: html.style.zoom || getComputedStyle(html).zoom,
			bodyZoom: document.body.style.zoom || getComputedStyle(document.body).zoom,
			htmlFontSize: getComputedStyle(html).fontSize,
			screen: window.screen.width + 'x' + window.screen.height,
			firstButton: r ? [r.left, r.top, r.width, r.height] : null
		};
	};
	BbProbe.send('uiscale_before', JSON.stringify(measure()));
	setTimeout(function ()
	{
		BbProbe.send('uiscale_after', JSON.stringify(measure()));
		BbProbe.send('golden_main_menu_ui150', BBSnapshot.take());
		BbProbe.done();
	}, 14000);
};
