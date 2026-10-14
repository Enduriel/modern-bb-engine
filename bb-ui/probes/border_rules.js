// Old-WebKit border images. ES5 only. Vanilla draws its
// separator lines with `border-image: url(line_metal.png) 0% 0% 100% 0%` in its style
// sheets, never with a border style or width: do the box's border widths (`medium`,
// 3px) stay, in layout too, as if the border had a style?
//
// Declared in a style sheet, as vanilla does, and measured a moment later (our host
// rewrites style sheets as they arrive). The original's answers are
// `tests/goldens/border_rules.json`; the `ui_parity` test runs this file in our page.
BbProbe.run = function ()
{
	var image = 'border-image: url("coui://gfx/ui/skin/line_metal.png") 0% 0% 100% 0%;';
	var cases = [
		['plain', ''],
		['image', image],
		['imageBorderBox', image + ' box-sizing: border-box;'],
		['imageWidth1', image + ' border-width: 1px;'],
		['imageBottomWidth0', image + ' border-bottom-width: 0;'],
		['imageOutset', image + ' border-image-outset: 0 0 5px 0;'],
		['styleSolid', 'border-style: solid;']
	];
	var css = '';
	for (var i = 0; i < cases.length; i++)
	{
		css += '.probe-border-' + cases[i][0] + ' { position: absolute; left: 0; top: 0; width: 100px; height: 20px; ' +
			cases[i][1] + ' }\n';
	}
	var sheet = document.createElement('style');
	sheet.textContent = css;
	document.head.appendChild(sheet);
	var boxes = [];
	for (var j = 0; j < cases.length; j++)
	{
		var box = document.createElement('div');
		box.className = 'probe-border-' + cases[j][0];
		document.body.appendChild(box);
		boxes.push(box);
	}
	(BbProbe.later || setTimeout)(function ()
	{
		var borders = {};
		for (var k = 0; k < cases.length; k++)
		{
			var el = boxes[k];
			var cs = getComputedStyle(el);
			borders[cases[k][0]] = el.offsetWidth + 'x' + el.offsetHeight + ' client ' + el.clientLeft + ',' + el.clientTop +
				' ' + el.clientWidth + 'x' + el.clientHeight + ' widths ' + cs.borderTopWidth + ' ' + cs.borderBottomWidth;
			document.body.removeChild(el);
		}
		document.head.removeChild(sheet);
		BbProbe.send('borders', JSON.stringify(borders));
		BbProbe.done();
	}, 1000);
};
