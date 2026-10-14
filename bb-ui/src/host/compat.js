// Injected into the page after bootstrap.js, before the page's own scripts.
//
// Makes Chromium lay out and report like Coherent GT's old WebKit where pages can
// tell the difference. Each rule was probed in the original.
// The font minimum is a browser preference instead (`host/app.rs`).
(function () {
	'use strict';

	// Coherent's built-in styles for form controls, where they differ from Chromium's:
	// controls don't get Chromium's 13.33px system font; checkboxes and radio buttons use
	// 16px Times New Roman with no margin; range sliders keep the page's font, with
	// `margin: 18px 0` and old WebKit's grey. `:where()` keeps these at zero specificity,
	// below every page or mod rule, as built-in styles are.
	// Coherent also never fakes a bold or an italic face: text asked to be bold or italic
	// in a family that has only a regular face (the game's fonts each have one: the
	// resource numbers are bold, dialog subtitles italic) draws in that face as it is,
	// where Chromium would embolden or slant it. Nor does it kern: "AV" in Cinzel-Bold,
	// which has a kerning table, is as wide as "A" and "V" apart there, even with
	// `font-kerning: normal` or `text-rendering: optimizeLegibility` asked for (probed,
	// `bb-ui/probes/text_rules.js`); Chromium kerns by default.
	var BUILT_IN_CSS =
		':where(input, select, textarea, button) { font: inherit; }\n' +
		':where(input[type=checkbox], input[type=radio]) { margin: 0; font: 16px "Times New Roman"; }\n' +
		':where(input[type=range]) { margin: 18px 0; color: #909090; }\n' +
		':where(:root) { font-synthesis-weight: none; font-synthesis-style: none; font-kerning: none; }\n';

	// Images with nothing to show paint nothing, as in Coherent: no `src` (the
	// character screen's talent stars), an empty one (every empty stash slot's
	// overlay), or one that failed to load. Where such an image has a size, Chromium
	// paints a broken-image icon in a grey box instead. A mask hides that and leaves
	// the image's box and what the mouse hits as they were. Failed loads are marked
	// below, as the image's error arrives.
	var BROKEN = 'data-bb-broken';
	var NOTHING_TO_SHOW_CSS =
		':where(img:not([src]), img[src=""], img[' + BROKEN + ']) ' +
		'{ mask-image: linear-gradient(transparent, transparent) !important; }\n';
	document.addEventListener('DOMContentLoaded', function () {
		var style = document.createElement('style');
		style.setAttribute('data-bb-compat', '');
		style.textContent = BUILT_IN_CSS + NOTHING_TO_SHOW_CSS;
		document.head.insertBefore(style, document.head.firstChild);
	});
	// Image loads don't bubble, but the document sees them on their way down to an image
	// in the page. The game's UI adds its images to the page before they load. An error
	// can arrive after the image has moved on to a file that loaded.
	document.addEventListener('error', function (e) {
		var image = e.target;
		if (image instanceof HTMLImageElement && image.complete && image.naturalWidth === 0) {
			image.setAttribute(BROKEN, '');
		}
	}, true);
	document.addEventListener('load', function (e) {
		if (e.target instanceof HTMLImageElement && e.target.hasAttribute(BROKEN)) {
			e.target.removeAttribute(BROKEN);
		}
	}, true);

	// Absolutely positioned boxes centred between their top and bottom insets by `auto`
	// margins. When the insets leave less room than the box (the world top bar's time
	// buttons: `top: 13.8rem; bottom: 0` in a 7.3rem box), CSS 2.1, which old WebKit
	// follows, lets the two margins go equally negative. Chromium follows css-position-3
	// instead and first clamps the room between the insets to zero, which puts the box
	// elsewhere (33-55px lower there). Rewriting such a rule's insets to
	// `(top - bottom) / 2` and its negation never leaves negative room, and centring then
	// lands the box exactly where CSS 2.1 does: (height of the containing block + top -
	// bottom - height of the box) / 2. Only whole rules that declare all of it are
	// rewritten, which is how the game's CSS writes them.
	function zeroWithUnit(value) {
		return value === '0' ? '0px' : value;
	}

	function centreLikeCss21(style) {
		var lengths = [style.top, style.bottom, style.height];
		if (style.position !== 'absolute' || style.marginTop !== 'auto' || style.marginBottom !== 'auto' ||
			lengths.some(function (v) { return v === '' || v === 'auto'; })) {
			return;
		}
		var top = zeroWithUnit(style.top);
		var bottom = zeroWithUnit(style.bottom);
		style.top = 'calc((' + top + ' - ' + bottom + ') / 2)';
		style.bottom = 'calc((' + bottom + ' - ' + top + ') / 2)';
	}

	// Percentages. Old WebKit laid out in whole pixels: a percentage resolved to a
	// length is cut down to whole pixels (`height: 12.2%` of 250px is 30px there, 30.5px
	// here), and stacked boxes don't drift by half pixels. CSS's `round()` does the same
	// in the declaration.
	var BOX_PROPERTIES = ['width', 'height', 'min-width', 'min-height', 'max-width', 'max-height',
		'top', 'right', 'bottom', 'left', 'margin-top', 'margin-right', 'margin-bottom', 'margin-left',
		'padding-top', 'padding-right', 'padding-bottom', 'padding-left'];

	function wholePixelPercentages(style) {
		for (var i = 0; i < BOX_PROPERTIES.length; i++) {
			var property = BOX_PROPERTIES[i];
			var value = style.getPropertyValue(property);
			if (/^-?[\d.]+%$/.test(value)) {
				style.setProperty(property, 'round(to-zero, ' + value + ', 1px)', style.getPropertyPriority(property));
			}
		}
	}

	// Border images. Old WebKit keeps a box's border widths (`medium`, 3px, unless
	// declared) while it has a border image, though its border style is `none`: they
	// take room, and the image is drawn in them (probed, `bb-ui/probes/border_rules.js`).
	// Vanilla draws all its separator lines so, `border-image: url(line_metal.png) 0% 0%
	// 100% 0%` and no style; the spec computes such widths to 0, so Chromium draws
	// nothing. A rule that sets a border image and no border style gets `solid` (drawn
	// transparent where the image doesn't show), which keeps the widths as old WebKit
	// does.
	var BORDER_STYLES = ['border-top-style', 'border-right-style', 'border-bottom-style', 'border-left-style'];

	function bordersUnderImages(style) {
		var source = style.getPropertyValue('border-image-source');
		if (source === '' || source === 'none' ||
			BORDER_STYLES.some(function (p) { return style.getPropertyValue(p) !== ''; })) {
			return;
		}
		var priority = style.getPropertyPriority('border-image-source');
		style.setProperty('border-style', 'solid', priority);
		if (style.getPropertyValue('border-top-color') === '') {
			style.setProperty('border-color', 'transparent', priority);
		}
	}

	function fixRules(rules) {
		for (var i = 0; i < rules.length; i++) {
			var rule = rules[i];
			if (rule.type === CSSRule.MEDIA_RULE || rule.type === CSSRule.SUPPORTS_RULE) {
				fixRules(rule.cssRules);
			} else if (rule.type === CSSRule.STYLE_RULE) {
				centreLikeCss21(rule.style);
				wholePixelPercentages(rule.style);
				bordersUnderImages(rule.style);
			}
		}
	}

	// Each style sheet once, as it arrives: the page's own, and those mods add later.
	var seenSheets = new WeakSet();
	function fixSheets() {
		for (var i = 0; i < document.styleSheets.length; i++) {
			var sheet = document.styleSheets[i];
			if (seenSheets.has(sheet)) {
				continue;
			}
			var rules;
			try {
				rules = sheet.cssRules;
			} catch (e) {
				continue; // not loaded yet
			}
			seenSheets.add(sheet);
			fixRules(rules);
		}
	}
	document.addEventListener('load', function (e) {
		if (e.target && (e.target.tagName === 'LINK' || e.target.tagName === 'STYLE')) {
			fixSheets();
		}
	}, true);
	document.addEventListener('DOMContentLoaded', fixSheets);

	// `scrollWidth`. Old WebKit's horizontal scrollable overflow held the boxes that have
	// an area (wider and taller than 0) and text, at any depth outside clipping boxes.
	// Chromium also counts in-flow children's margin boxes, empty or not. So an empty
	// row (0px tall because its children float) wider than its list, or a child's right
	// margin, makes a list scroll sideways only here, and aciScrollBar then adds a
	// horizontal bar. Vertically the two agree (the in-flow content height counts), so
	// `scrollHeight` stays native.
	//
	// Leaving boxes out can only shrink the overflow, so the native value stands when
	// nothing overflows: aciScrollBar reads this every 200ms for every list. Otherwise
	// the content is measured the old way, never beyond the native value.
	var scrollWidth = Object.getOwnPropertyDescriptor(Element.prototype, 'scrollWidth');

	function hasArea(rect) {
		return rect.width > 0 && rect.height > 0;
	}

	// The right edge (viewport x) of what old WebKit counted inside `element`. Stops
	// looking once it reaches `limit`.
	function contentRight(element, limit) {
		var right = -Infinity;
		var range = null;
		function visit(node, positioned) {
			for (var child = node.lastChild; child && right < limit; child = child.previousSibling) {
				if (child.nodeType === Node.TEXT_NODE) {
					range = range || document.createRange();
					range.selectNodeContents(child);
					var text = range.getBoundingClientRect();
					if (hasArea(text)) {
						right = Math.max(right, text.right);
					}
					continue;
				}
				if (child.nodeType !== Node.ELEMENT_NODE) {
					continue;
				}
				var style = getComputedStyle(child);
				// Not laid out, or laid out against something outside `element`.
				if (style.display === 'none' || style.position === 'fixed' ||
					(style.position === 'absolute' && !positioned)) {
					continue;
				}
				var box = child.getBoundingClientRect();
				if (hasArea(box)) {
					right = Math.max(right, box.right);
				}
				if (style.overflowX === 'visible') {
					visit(child, positioned || style.position !== 'static');
				}
			}
		}
		visit(element, getComputedStyle(element).position !== 'static');
		return right;
	}

	Object.defineProperty(Element.prototype, 'scrollWidth', {
		configurable: true,
		enumerable: scrollWidth.enumerable,
		get: function () {
			var native = scrollWidth.get.call(this);
			var client = this.clientWidth;
			// The viewport's scrolling follows other rules; nothing here relies on it.
			if (native <= client || this === document.documentElement || this === document.body) {
				return native;
			}
			var origin = this.getBoundingClientRect().left + this.clientLeft - this.scrollLeft;
			var right = contentRight(this, origin + native);
			return Math.max(client, Math.min(native, Math.round(right - origin)));
		}
	});
})();
