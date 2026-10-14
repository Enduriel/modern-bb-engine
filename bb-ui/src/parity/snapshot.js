// UI parity snapshot. The same script runs in the original game (through a probe
// runner) and in our Chromium host, so their outputs compare directly.
// ES5 only: the original's Coherent GT is a 2015 WebKit.
//
// BBSnapshot.take(root) -> JSON text of the visible element tree under `root` (default
// document.body). Each node:
//   { tag, cls, id?, procedural?, text?, rect: [x, y, w, h], style: {prop: value}, children: [...] }
// (`procedural`: an image the engine draws, from coui://procedural/.)
// Elements with computed `display: none` are left out (a hidden subtree is not compared).
// Rects are rounded to whole pixels. `style` holds only the properties in STYLE, which
// both engines compute; absent or engine-specific properties (-coherent-*, …) are left
// out, so the comparison is about what a player sees.
var BBSnapshot = (function ()
{
	var STYLE = [
		'position', 'visibility', 'opacity', 'z-index', 'overflow-x', 'overflow-y',
		'color', 'background-color', 'background-image', 'background-position',
		'background-size', 'background-repeat',
		'font-family', 'font-size', 'font-weight', 'font-style', 'line-height',
		'letter-spacing', 'text-align', 'text-transform', 'white-space', 'text-shadow',
		'margin-top', 'margin-right', 'margin-bottom', 'margin-left',
		'padding-top', 'padding-right', 'padding-bottom', 'padding-left',
		'border-top-width', 'border-right-width', 'border-bottom-width', 'border-left-width',
		'border-top-style', 'border-top-color', 'transform', 'box-sizing',
		// Not in the goldens (taken before it was added): it tells the comparison which
		// of our boxes have a border image.
		'border-image-source'
	];

	function walk(el)
	{
		var cs = window.getComputedStyle(el);
		if (cs === null || cs.display === 'none')
		{
			return null;
		}
		var node = { tag: el.tagName.toLowerCase(), cls: el.className && typeof el.className === 'string' ? el.className : '' };
		if (el.id)
		{
			node.id = el.id;
		}
		// Images the engine draws (portraits): their size follows what they show.
		if (el.tagName === 'IMG' && (el.getAttribute('src') || '').indexOf('coui://procedural/') === 0)
		{
			node.procedural = true;
		}
		var text = '';
		for (var c = el.firstChild; c !== null; c = c.nextSibling)
		{
			if (c.nodeType === 3)
			{
				text += c.nodeValue;
			}
		}
		text = text.replace(/\s+/g, ' ').replace(/^ | $/g, '');
		if (text !== '')
		{
			node.text = text;
		}
		var r = el.getBoundingClientRect();
		node.rect = [Math.round(r.left), Math.round(r.top), Math.round(r.width), Math.round(r.height)];
		node.style = {};
		node.style.display = cs.display;
		for (var i = 0; i < STYLE.length; i++)
		{
			var value = cs.getPropertyValue(STYLE[i]);
			if (value !== null && value !== '')
			{
				node.style[STYLE[i]] = value;
			}
		}
		node.children = [];
		for (var k = 0; k < el.children.length; k++)
		{
			var child = walk(el.children[k]);
			if (child !== null)
			{
				node.children.push(child);
			}
		}
		return node;
	}

	return {
		take: function (_root)
		{
			return JSON.stringify({
				viewport: [window.innerWidth, window.innerHeight],
				root: walk(_root || document.body)
			});
		}
	};
})();
