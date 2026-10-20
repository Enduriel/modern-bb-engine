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

	// The UI scale. Coherent zooms the page: it lays out at the screen size divided by the
	// scale and is drawn at the screen's resolution, but `devicePixelRatio` stays 1
	// (probed). Ours is drawn so through the browser's device scale factor,
	// which Chromium reports there. The real one (the scale) stays readable to the
	// rewrites below, as `--bb-zoom` on the root, kept up to date as the scale changes
	// (which resizes the page).
	var dprSource = Object.getOwnPropertyDescriptor(window, 'devicePixelRatio') ||
		Object.getOwnPropertyDescriptor(Window.prototype, 'devicePixelRatio');
	var screenPixels = function () {
		return dprSource && dprSource.get ? dprSource.get.call(window) : 1;
	};
	var zoomStyle = document.createElement('style');
	zoomStyle.setAttribute('data-bb-compat', 'zoom');
	function trackZoom() {
		zoomStyle.textContent = ':root { --bb-zoom: ' + screenPixels() + '; }';
	}
	window.addEventListener('resize', trackZoom);
	Object.defineProperty(window, 'devicePixelRatio', {
		configurable: true,
		enumerable: true,
		get: function () { return 1; }
	});

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
		trackZoom();
		document.head.insertBefore(zoomStyle, style.nextSibling);
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

	// No native drags. Chromium starts dragging an image (or a link, or selected text)
	// away once a press on it moves a few pixels, and then the release is no click: a
	// shaky click on any of vanilla's image buttons (end turn, the top bar's) was lost.
	// The game's UI drags with mouse events only (jquery.event.drag, which keeps the
	// native drag off its own items), and neither vanilla nor the mods surveyed use
	// HTML5 drag and drop, which Coherent's offscreen views have no system drag for.
	// Unprobed in the original (a probe can't press its mouse); the press moves on as
	// mouse events, as in the game's own drags.
	window.addEventListener('dragstart', function (e) {
		e.preventDefault();
	}, true);

	// The keys Coherent's table has no code for (the function keys) came to the page as
	// keyCode -1 (probed). The host sends -1 too, which Chromium keeps as 16 bits.
	['keyCode', 'which'].forEach(function (name) {
		var native = Object.getOwnPropertyDescriptor(KeyboardEvent.prototype, name) ||
			Object.getOwnPropertyDescriptor(UIEvent.prototype, name);
		if (!native || !native.get) {
			return;
		}
		Object.defineProperty(KeyboardEvent.prototype, name, {
			configurable: true,
			enumerable: native.enumerable,
			get: function () {
				var code = native.get.call(this);
				return code === 0xFFFF ? -1 : code;
			}
		});
	});

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
	//
	// The half pixel. Old WebKit lays out in whole screen pixels: of an odd room it gives
	// the start margin the smaller part, rounding the half toward zero (4px of 9; -5 of
	// -11), where Chromium places the box at the half and draws it at the next pixel on.
	// Vanilla centres ~270 boxes so vertically and ~150 horizontally (button labels,
	// icons), so each sat a pixel off whenever its room was odd (probed at UI scales 1,
	// 1.25 and 1.5, `bb-ui/probes/centring_rules.js`). A quarter screen pixel back toward
	// the start makes Chromium's rounding land where old WebKit's does when there is room
	// to spare (with less, Chromium's rounding of the half already agrees, and across the
	// start margin is 0 in both). Whether there is room is told from the rule's own size.
	function zeroWithUnit(value) {
		return value === '0' ? '0px' : value;
	}

	// A quarter pixel back when the room `100% - start - end - size` is positive, else 0;
	// at UI scale 1 only. Under a zoom, old WebKit first cuts every zoomed length down to
	// whole screen pixels (a 91px box is 113 of them at 1.25, so its room in a 125px
	// parent is 12, not 11.25), which a quarter pixel can't follow: there the nudge put
	// the menu and half of the probe's boxes a pixel further off.
	function halfPixelBack(start, end, size) {
		return 'calc(-0.25px * max(0, sign(100% - ' + start + ' - ' + end + ' - ' + size + ')) * ' +
			'max(0, 1 - abs(var(--bb-zoom, 1) - 1) * 1000))';
	}

	function centreLikeCss21(style) {
		var lengths = [style.top, style.bottom, style.height];
		if (style.position !== 'absolute' || style.marginTop !== 'auto' || style.marginBottom !== 'auto' ||
			lengths.some(function (v) { return v === '' || v === 'auto'; })) {
			return;
		}
		var top = zeroWithUnit(style.top);
		var bottom = zeroWithUnit(style.bottom);
		var back = halfPixelBack(top, bottom, zeroWithUnit(style.height));
		style.top = 'calc((' + top + ' - ' + bottom + ') / 2 + ' + back + ')';
		style.bottom = 'calc((' + bottom + ' - ' + top + ') / 2 - ' + back + ')';
	}

	// The same box centred between its left and right insets: Chromium and old WebKit
	// agree but for the half pixel.
	function centreAcrossLikeOldWebKit(style) {
		var lengths = [style.left, style.right, style.width];
		if (style.position !== 'absolute' || style.marginLeft !== 'auto' || style.marginRight !== 'auto' ||
			lengths.some(function (v) { return v === '' || v === 'auto'; })) {
			return;
		}
		var left = zeroWithUnit(style.left);
		var right = zeroWithUnit(style.right);
		var back = halfPixelBack(left, right, zeroWithUnit(style.width));
		style.left = 'calc(' + left + ' + ' + back + ')';
		style.right = 'calc(' + right + ' - ' + back + ')';
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

	// Blurred text shadows. INTENTIONAL DIVERGENCE (docs/differences.md, "Blurred text
	// shadows in the UI"): this changes what scripts read, to draw what Coherent draws.
	// Coherent draws a blurred shadow lighter and wider than Chromium: vanilla's
	// `1px 1px 1px #000` never reaches black there (darkest 11-15 luminance below a dark
	// panel against Chromium's 24-29), and a wider blur alone still peaks darker. Fitted
	// on a test card screenshotted in the original (`bb-ui/probes/text_card.js`, the game's
	// fonts and shadows): twice the blur at 0.65 of the alpha draws closest (Coherent
	// seemingly takes the blur radius as the Gaussian's sigma, where Chromium takes half
	// of it). Shadows without a blur (`1px 1px #A36426, -1px -1px #000`) already draw the
	// same and are left as they are.
	// What scripts see: a rule's `text-shadow` and the computed style read `rgba(0, 0, 0,
	// 0.65) 1px 1px 2px` where the original reports `rgb(0, 0, 0) 1px 1px 1px`. Only
	// style sheets are rewritten: no vanilla or mod script sets a shadow inline.
	var SHADOW_BLUR = 2;
	var SHADOW_ALPHA = 0.65;
	var colourCanvas = null;

	// `colour` as [r, g, b, a] (any CSS colour, through a canvas's own parser).
	function rgba(colour) {
		colourCanvas = colourCanvas || document.createElement('canvas').getContext('2d');
		colourCanvas.fillStyle = '#000';
		colourCanvas.fillStyle = colour;
		var c = colourCanvas.fillStyle;
		if (c.charAt(0) === '#') {
			return [parseInt(c.substr(1, 2), 16), parseInt(c.substr(3, 2), 16), parseInt(c.substr(5, 2), 16), 1];
		}
		var parts = c.replace(/^rgba?\(|\)$/g, '').split(',');
		return [+parts[0], +parts[1], +parts[2], parts.length > 3 ? +parts[3] : 1];
	}

	function lighterShadow(shadow) {
		var lengths = shadow.match(/-?[\d.]+px|(^|\s)0(?=\s|$)/g) || [];
		var blur = lengths.length > 2 ? parseFloat(lengths[2]) : 0;
		if (!(blur > 0)) {
			return shadow;
		}
		var colour = shadow.replace(/-?[\d.]+px|(^|\s)0(?=\s|$)/g, ' ').trim();
		var c = rgba(colour === '' ? 'currentcolor' : colour);
		return 'rgba(' + c[0] + ', ' + c[1] + ', ' + c[2] + ', ' + Math.round(c[3] * SHADOW_ALPHA * 1000) / 1000 + ') ' +
			lengths[0].trim() + ' ' + lengths[1].trim() + ' ' + (blur * SHADOW_BLUR) + 'px';
	}

	function lighterBlurredShadows(style) {
		var value = style.getPropertyValue('text-shadow');
		if (value === '' || value === 'none' || value.indexOf('var(') !== -1) {
			return;
		}
		var shadows = value.split(/,(?![^(]*\))/);
		var lighter = shadows.map(function (s) { return lighterShadow(s.trim()); });
		if (lighter.join() !== shadows.map(function (s) { return s.trim(); }).join()) {
			style.setProperty('text-shadow', lighter.join(', '), style.getPropertyPriority('text-shadow'));
		}
	}

	function fixRules(rules) {
		for (var i = 0; i < rules.length; i++) {
			var rule = rules[i];
			if (rule.type === CSSRule.MEDIA_RULE || rule.type === CSSRule.SUPPORTS_RULE) {
				fixRules(rule.cssRules);
			} else if (rule.type === CSSRule.STYLE_RULE) {
				centreLikeCss21(rule.style);
				centreAcrossLikeOldWebKit(rule.style);
				wholePixelPercentages(rule.style);
				bordersUnderImages(rule.style);
				lighterBlurredShadows(rule.style);
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
	// nothing overflows: aciScrollBar reads this on every update of every list. Otherwise
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
