// Old-WebKit layout rules that the parity goldens hinted at. ES5 only.
//  1. Small font sizes: which are clamped to 8px, and does layout (em, text height) use the clamp?
//  2. Scrollable overflow: do empty (zero-width or zero-height) boxes count toward scrollWidth/Height?
// (Text widths: `text_rules.js`; border images: `border_rules.js`; centring by `auto`
// margins: `centring_rules.js`.)
//
// The original's answers are `tests/goldens/layout_rules.json`. The `ui_parity` test
// runs this file in our page, with a stand-in `BbProbe`, and compares.
BbProbe.run = function ()
{
	var fonts = {};
	var host = document.createElement('div');
	host.style.fontSize = '10px';
	document.body.appendChild(host);
	var cases = [['px6', 'font-size: 6px'], ['px7', 'font-size: 7px'], ['pct50', 'font-size: 50%'],
		['em05', 'font-size: 0.5em'], ['rem05', 'font-size: 0.5rem'], ['small', 'font-size: x-small'],
		['xxsmall', 'font-size: xx-small'], ['pt5', 'font-size: 5pt'], ['px1', 'font-size: 1px'], ['px0', 'font-size: 0px']];
	for (var i = 0; i < cases.length; i++)
	{
		var el = document.createElement('span');
		el.setAttribute('style', cases[i][1]);
		el.textContent = 'Xg';
		host.appendChild(el);
		fonts[cases[i][0]] = getComputedStyle(el).fontSize + ' h=' + el.getBoundingClientRect().height;
	}
	fonts.html = getComputedStyle(document.documentElement).fontSize;
	fonts.body = getComputedStyle(document.body).fontSize;
	// Under body (62.5% of 62.5% of medium, relative all the way): text, em widths, and percentages of it.
	var plain = document.createElement('span');
	plain.textContent = 'Xg';
	document.body.appendChild(plain);
	fonts.bodyText = 'h=' + plain.getBoundingClientRect().height + ' w=' + plain.getBoundingClientRect().width;
	var em = document.createElement('div');
	em.setAttribute('style', 'width: 10em; height: 1px; position: absolute');
	document.body.appendChild(em);
	fonts.bodyEm10 = em.getBoundingClientRect().width;
	var half = document.createElement('div');
	half.setAttribute('style', 'font-size: 50%; width: 10em; height: 1px; position: absolute');
	document.body.appendChild(half);
	fonts.bodyPct50 = getComputedStyle(half).fontSize + ' em10=' + half.getBoundingClientRect().width;
	var big = document.createElement('div');
	big.setAttribute('style', 'font-size: 200%; width: 10em; height: 1px; position: absolute');
	document.body.appendChild(big);
	fonts.bodyPct200 = getComputedStyle(big).fontSize + ' em10=' + big.getBoundingClientRect().width;
	BbProbe.send('fonts', JSON.stringify(fonts));

	// Each case: a 100x100 overflow:hidden box holding `html`; report scrollWidth x scrollHeight.
	var overflow = {};
	var boxes = [
		['wideZeroHeight', '<div style="width:150px;height:0"></div>'],
		['wideOnePx', '<div style="width:150px;height:1px"></div>'],
		['tallZeroWidth', '<div style="width:0;height:150px"></div>'],
		['emptyWithFloats', '<div style="width:150px"><div style="float:left;width:20px;height:20px"></div></div>'],
		['emptyWithWideFloat', '<div style="width:50px"><div style="float:left;width:150px;height:20px"></div></div>'],
		['zeroHeightBorder', '<div style="width:150px;height:0;border-top:1px solid red"></div>'],
		['zeroHeightPadding', '<div style="width:150px;height:0;padding-top:1px"></div>'],
		['zeroHeightHidden', '<div style="width:150px;height:0;overflow:hidden"></div>'],
		['absZeroHeight', '<div style="position:absolute;left:0;top:0;width:150px;height:0"></div>'],
		['relZeroHeightShifted', '<div style="position:relative;left:200px;width:10px;height:0"></div>'],
		['relSizedShifted', '<div style="position:relative;left:200px;width:10px;height:10px"></div>'],
		['zeroBoth', '<div style="position:relative;left:200px;top:200px;width:0;height:0"></div>'],
		['nowrapText', '<div style="white-space:nowrap;font-size:20px">XXXXXXXXXXXXXXXXXXXXXXXX</div>'],
		['emptyInline', '<span style="display:inline-block;width:150px;height:0"></span>'],
		['marginRight', '<div style="width:90px;height:10px;margin-right:100px"></div>'],
		['nestedZero', '<div style="width:50px;height:0"><div style="width:150px;height:0"></div></div>'],
		['nestedSized', '<div style="width:50px;height:0"><div style="width:150px;height:10px"></div></div>'],
		['clippedChild', '<div style="width:50px;height:50px;overflow:hidden"><div style="width:150px;height:10px"></div></div>'],
		['transformed', '<div style="width:50px;height:10px;-webkit-transform:translateX(120px)"></div>']
	];
	for (var j = 0; j < boxes.length; j++)
	{
		var box = document.createElement('div');
		box.setAttribute('style', 'position:absolute;left:0;top:0;width:100px;height:100px;overflow:hidden');
		box.innerHTML = boxes[j][1];
		document.body.appendChild(box);
		overflow[boxes[j][0]] = box.scrollWidth + 'x' + box.scrollHeight;
		document.body.removeChild(box);
	}
	BbProbe.send('overflow', JSON.stringify(overflow));

	BbProbe.done();
};
