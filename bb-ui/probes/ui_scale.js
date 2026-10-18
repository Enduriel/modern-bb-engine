// The UI scale probe, the page half (see ui_scale.nut): the page's geometry before the
// scripts set the UI scale (at 8 s) and after, plus a snapshot of the main menu after
// (golden_main_menu_<SCALE_NAME>). ES5 only; needs BBSnapshot (src/parity/snapshot.js)
// prepended.
var SCALE_NAME = 'ui150'; // ui_scale.nut's scale, times 100
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
		BbProbe.send('golden_main_menu_' + SCALE_NAME, BBSnapshot.take());
		BbProbe.done();
	}, 14000);
};
