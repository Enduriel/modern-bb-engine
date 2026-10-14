// The page half (see over_ui.nut): hide the main menu and cover the
// page with eight full-height test stripes, then keep them while the scripts ask. ES5
// only.
BbProbe.run = function ()
{
	$('.main-menu-screen').css('display', 'none');
	var grey = 'background: #808080;';
	var stripes = [
		'',
		grey,
		grey + ' pointer-events: none;',
		'background: transparent;',
		'background: rgba(0, 0, 0, 0.3);',
		grey + ' opacity: 0.01;',
		grey + ' opacity: 0;',
		grey + ' visibility: hidden;'
	];
	for (var i = 1; i < stripes.length; i++)
	{
		var el = document.createElement('div');
		el.setAttribute('style', 'position: absolute; top: 0; height: 100%; width: 320px; z-index: 10000; left: ' + (320 * i) + 'px; ' + stripes[i]);
		document.body.appendChild(el);
	}
	console.error('PROBE js stripes up');
	// The scripts ask 10 s after they ran (just before this).
	setTimeout(function () { BbProbe.done(); }, 20000);
};
