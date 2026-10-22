// Injected into the page after compat.js, before the page's own scripts.
//
// Tiled backgrounds draw upside down, as in Coherent (probed in the original:
// `bb-ui/probes/background_tiles.js`). Where a box's background image repeats
// along an axis and its first tile, where it is placed, doesn't reach across the box
// along it, Coherent draws every tile upside down (not mirrored sideways); where the
// tile covers the box along each axis it repeats in, or the image doesn't repeat, it
// draws the picture as it is. The main menu's buttons are such boxes:
// `button_04_default.png` is 43px tall in a 46px box, so the original draws its gold
// edges 4px higher and its light edge at the top.
//
// How: each style-sheet rule that sets a background image gets a twin right after it,
// for marked boxes only, with the same images upside down (`coui://…png?bb-upside-
// down`, turned by the `coui://` scheme):
//
//     .a .b { background-image: url(x.png); }
//     .a .b:where([data-bb-tiled]) { background-image: url(x.png?bb-upside-down); }
//
// `:where()` adds nothing to the twin's specificity and it comes right after the rule,
// so it wins where the rule would and loses to whatever beats the rule. The rule keeps
// its own `url()`, so the style sheet still holds its picture once used, as before (a
// picture named through a `var()` would be fetched again on each return to a screen).
// The marks follow the boxes: the ones with a background image are watched for size
// changes, and checked again as the page changes them.
//
// Cheaply, for screens of thousands of boxes (the character screen's stash): a box is
// only looked at when a rule that sets a background image may apply to it (by its
// selector's last id, class or tag, `mayHaveImage`); it is checked after the frame's
// own layout, when the size observer reports it (a box the page changed is observed
// anew for that), never by working out the page's style and layout early; a box
// without an area is left until it has one (it draws no background); and a batch of
// checks reads every box before it changes any mark.
//
// What scripts see: a marked box's computed `background-image` names the turned
// picture and the box carries the mark; style sheets hold the twins. Not emulated:
// images set in `style` attributes (no vanilla or mod script sets one), on
// pseudo-elements, and in rules that only apply under `:hover` and the like (the mark
// is set from the box's style as it is checked). Gradients are drawn as they are (not
// probed).
(function () {
	'use strict';

	var FLAG = 'data-bb-tiled';
	var QUERY = 'bb-upside-down';

	// ---- Style sheets: every rule's background images, also upside down. ----

	// `url(...)` of a picture the scheme can turn: not a `data:` or `blob:` one.
	var URL_PATTERN = /url\((['"]?)(.*?)\1\)/g;

	function turned(url) {
		if (/^(data|blob):/i.test(url)) {
			return url;
		}
		var hash = url.indexOf('#');
		var base = hash === -1 ? url : url.slice(0, hash);
		return base + (base.indexOf('?') === -1 ? '?' : '&') + QUERY + (hash === -1 ? '' : url.slice(hash));
	}

	// The selectors of a selector list, split at its top-level commas.
	function selectors(list) {
		var parts = [];
		var depth = 0, quote = null, start = 0;
		for (var i = 0; i < list.length; i++) {
			var c = list.charAt(i);
			if (quote !== null) {
				if (c === '\\') {
					i++;
				} else if (c === quote) {
					quote = null;
				}
			} else if (c === '"' || c === "'") {
				quote = c;
			} else if (c === '(' || c === '[') {
				depth++;
			} else if (c === ')' || c === ']') {
				depth--;
			} else if (c === ',' && depth === 0) {
				parts.push(list.slice(start, i).trim());
				start = i + 1;
			}
		}
		parts.push(list.slice(start).trim());
		return parts;
	}

	var PSEUDO_ELEMENT = /::|:(before|after|first-line|first-letter)\b/i;

	// The twin of a style rule that sets a background image, or null.
	function upsideDownTwin(rule) {
		var value = rule.style.getPropertyValue('background-image');
		if (value.indexOf('url(') === -1 || value.indexOf('var(') !== -1) {
			return null;
		}
		var boxes = selectors(rule.selectorText).filter(function (s) {
			return s !== '' && !PSEUDO_ELEMENT.test(s);
		});
		if (boxes.length === 0) {
			return null;
		}
		var flipped = value.replace(URL_PATTERN, function (all, quote, url) {
			return 'url("' + turned(url).replace(/"/g, '\\"') + '")';
		});
		var important = rule.style.getPropertyPriority('background-image') === 'important' ? ' !important' : '';
		return boxes.map(function (s) { return s + ':where([' + FLAG + '])'; }).join(', ') +
			' { background-image: ' + flipped + important + '; }';
	}

	function fixRules(parent) {
		var rules = parent.cssRules;
		for (var i = 0; i < rules.length; i++) {
			var rule = rules[i];
			if (rule.type === CSSRule.MEDIA_RULE || rule.type === CSSRule.SUPPORTS_RULE) {
				fixRules(rule);
			} else if (rule.type === CSSRule.STYLE_RULE) {
				var twin = upsideDownTwin(rule);
				if (twin !== null) {
					try {
						parent.insertRule(twin, i + 1);
						i++;
					} catch (e) {
						// A selector this engine can't take a `:where()` after: drawn as it is.
					}
				}
			}
		}
	}

	// ---- Which boxes a rule with a background image may apply to. ----

	// The ids (`#a`), classes (`.a`) and tags (`a`) the last part of a selector that sets
	// a background image names; `all`: some such selector names none of them (`*`,
	// `[open]`, `:hover`), or couldn't be read, so every box may have one.
	var imageKeys = { ids: new Set(), classes: new Set(), tags: new Set(), all: false };

	// The classes named anywhere in a selector that sets a background image: a box's
	// class changing can change a background only by one of them (`anyClass`: a
	// selector reads the class attribute some other way, `[class*=…]`).
	var imageClasses = { names: new Set(), anyClass: false };
	var CLASS_NAME = /\.([\w\u0080-\uffff-]+)/g;

	// A `style` that names a background or sets a custom property (which a rule's
	// `var()` may read).
	var BACKGROUND_STYLE = /background|--/i;

	// What a selector's subject (its last compound selector) must be: `#id`, `.class`
	// or a tag, or null for none of those outside parentheses and brackets.
	function subjectKey(selector) {
		if (selector.indexOf('\\') !== -1 || selector.indexOf('|') !== -1) {
			return null;
		}
		var flat = '';
		var depth = 0, quote = null;
		for (var i = 0; i < selector.length; i++) {
			var c = selector.charAt(i);
			if (quote !== null) {
				if (c === quote) {
					quote = null;
				}
			} else if (c === '"' || c === "'") {
				quote = c;
			} else if (c === '(' || c === '[') {
				depth++;
			} else if (c === ')' || c === ']') {
				depth--;
				flat += depth === 0 ? ':' : '';
			} else if (depth === 0) {
				flat += c;
			}
		}
		var parts = flat.trim().split(/\s*[\s>+~]\s*/);
		var subject = parts[parts.length - 1];
		var key = /#([\w\u0080-\uffff-]+)/.exec(subject) || /\.([\w\u0080-\uffff-]+)/.exec(subject);
		if (key !== null) {
			return key[0];
		}
		key = /^[a-zA-Z][\w-]*/.exec(subject);
		return key === null ? null : key[0].toLowerCase();
	}

	function noteImages(parent) {
		var rules = parent.cssRules;
		for (var i = 0; i < rules.length; i++) {
			var rule = rules[i];
			if (rule.type === CSSRule.STYLE_RULE) {
				var value = rule.style.getPropertyValue('background-image');
				if (value === '' || value === 'none') {
					continue;
				}
				selectors(rule.selectorText).forEach(function (s) {
					if (s === '' || PSEUDO_ELEMENT.test(s)) {
						return;
					}
					if (s.indexOf('\\') !== -1 || /\[\s*class/i.test(s)) {
						imageClasses.anyClass = true;
					}
					var named;
					var unquoted = s.replace(/"[^"]*"|'[^']*'/g, '');
					while ((named = CLASS_NAME.exec(unquoted)) !== null) {
						imageClasses.names.add(named[1]);
					}
					var key = subjectKey(s);
					if (key === null) {
						imageKeys.all = true;
					} else if (key.charAt(0) === '#') {
						imageKeys.ids.add(key.slice(1));
					} else if (key.charAt(0) === '.') {
						imageKeys.classes.add(key.slice(1));
					} else {
						imageKeys.tags.add(key);
					}
				});
			} else if (rule.cssRules) {
				noteImages(rule);
			} else if (rule.type === CSSRule.IMPORT_RULE) {
				try {
					noteImages(rule.styleSheet);
				} catch (e) {
					imageKeys.all = true;
				}
			}
		}
	}

	// Whether a rule with a background image (or the box's own `style`) may give
	// `element` one.
	function mayHaveImage(element) {
		if (imageKeys.all) {
			return true;
		}
		if (imageKeys.tags.has(element.localName.toLowerCase()) || (element.id && imageKeys.ids.has(element.id))) {
			return true;
		}
		var classes = element.classList;
		for (var i = 0; i < classes.length; i++) {
			if (imageKeys.classes.has(classes[i])) {
				return true;
			}
		}
		var own = element.getAttribute('style');
		return own !== null && BACKGROUND_STYLE.test(own);
	}

	var seenSheets = new WeakSet();
	function fixSheets() {
		var fixed = false;
		for (var i = 0; i < document.styleSheets.length; i++) {
			var sheet = document.styleSheets[i];
			if (seenSheets.has(sheet)) {
				continue;
			}
			try {
				sheet.cssRules.length;
			} catch (e) {
				continue; // not loaded yet
			}
			seenSheets.add(sheet);
			fixRules(sheet);
			try {
				noteImages(sheet);
			} catch (e) {
				imageKeys.all = true;
			}
			fixed = true;
		}
		return fixed;
	}

	// ---- The marks: which boxes tile their background. ----

	// The natural sizes of pictures, by URL, once loaded; the boxes waiting for one.
	var sizes = new Map();
	var waiting = new Map();

	function naturalSize(url, element) {
		var size = sizes.get(url);
		if (size !== undefined) {
			return size;
		}
		var boxes = waiting.get(url);
		if (boxes === undefined) {
			boxes = new Set();
			waiting.set(url, boxes);
			var image = new Image();
			image.onload = image.onerror = function () {
				sizes.set(url, [image.naturalWidth, image.naturalHeight]);
				var those = waiting.get(url);
				waiting.delete(url);
				those.forEach(watch);
			};
			image.src = url;
		}
		boxes.add(element);
		return null;
	}

	// The first of a computed list's layers.
	function first(value) {
		return value.split(/,(?![^(]*\))/)[0].trim();
	}

	// A computed position (`0%`, `-10px`, `calc(50% - 10px)`) in a room of `room` px.
	function position(value, room) {
		var total = 0;
		var term = /([+-]?)\s*(-?[\d.]+)(%|px)/g;
		var m;
		while ((m = term.exec(value)) !== null) {
			var n = parseFloat(m[2]) * (m[1] === '-' ? -1 : 1);
			total += m[3] === '%' ? n / 100 * room : n;
		}
		return total;
	}

	// The tile's size from a computed `background-size`, in an area `w` x `h`; null when
	// it waits for the picture.
	function tileSize(value, url, w, h, element) {
		if (value === 'cover' || value === 'contain' || value.indexOf('auto') !== -1 || value.split(' ').length < 2) {
			var natural = naturalSize(url, element);
			if (natural === null || natural[0] === 0 || natural[1] === 0) {
				return null;
			}
			if (value === 'cover' || value === 'contain') {
				var scale = (value === 'cover' ? Math.max : Math.min)(w / natural[0], h / natural[1]);
				return [natural[0] * scale, natural[1] * scale];
			}
		}
		var parts = value.split(' ');
		var length = function (v, room) {
			return /%$/.test(v) ? parseFloat(v) / 100 * room : parseFloat(v);
		};
		var tw = parts[0] === 'auto' ? null : length(parts[0], w);
		var th = parts.length < 2 || parts[1] === 'auto' ? null : length(parts[1], h);
		if (tw === null && th === null) {
			return natural;
		}
		if (tw === null) {
			tw = th * natural[0] / natural[1];
		} else if (th === null) {
			th = tw * natural[1] / natural[0];
		}
		return isNaN(tw) || isNaN(th) ? null : [tw, th];
	}

	// The box `name` (`border-box`, `padding-box`, `content-box`) as [x, y, w, h],
	// relative to the border box.
	function box(style, element, name) {
		var b = [style.borderLeftWidth, style.borderTopWidth, style.borderRightWidth, style.borderBottomWidth].map(parseFloat);
		var p = [style.paddingLeft, style.paddingTop, style.paddingRight, style.paddingBottom].map(parseFloat);
		var x = 0, y = 0, w = element.offsetWidth, h = element.offsetHeight;
		if (name !== 'border-box') {
			x += b[0]; y += b[1]; w -= b[0] + b[2]; h -= b[1] + b[3];
		}
		if (name === 'content-box') {
			x += p[0]; y += p[1]; w -= p[0] + p[2]; h -= p[1] + p[3];
		}
		return [x, y, w, h];
	}

	// Which axes a computed `background-repeat` layer repeats along, [x, y].
	function repeats(value) {
		var words = value.split(' ');
		if (words.length === 2) {
			return [words[0] !== 'no-repeat', words[1] !== 'no-repeat'];
		}
		return [value !== 'no-repeat' && value !== 'repeat-y', value !== 'no-repeat' && value !== 'repeat-x'];
	}

	// Whether `element`'s first background image tiles: along an axis it repeats in, its
	// first tile doesn't reach across the border box. Coherent measures against the
	// border box whatever `background-clip` says (probed: a tile that covers the padding
	// box inside a border still draws upside down).
	function tiles(element, style) {
		var match = /url\((['"]?)(.*?)\1\)/.exec(first(style.backgroundImage));
		var repeat = repeats(first(style.backgroundRepeat));
		if (match === null || !(repeat[0] || repeat[1])) {
			return false;
		}
		var url = match[2].replace(new RegExp('[?&]' + QUERY + '(?=$|[&#])'), '');
		var area = box(style, element, 'border-box');
		var origin = box(style, element, first(style.backgroundOrigin));
		if (area[2] <= 0 || area[3] <= 0) {
			return false;
		}
		var tile = tileSize(first(style.backgroundSize), url, origin[2], origin[3], element);
		if (tile === null || !(tile[0] > 0 && tile[1] > 0)) {
			return false;
		}
		var x = origin[0] + position(first(style.backgroundPositionX), origin[2] - tile[0]);
		var y = origin[1] + position(first(style.backgroundPositionY), origin[3] - tile[1]);
		var e = 0.01;
		return (repeat[0] && !(x <= e && x + tile[0] >= area[2] - e)) ||
			(repeat[1] && !(y <= e && y + tile[1] >= area[3] - e));
	}

	// Boxes with a background image, watched for size changes (which report each box
	// once it has a size, after layout and before it is painted).
	var watched = new WeakSet();

	// The marks a batch of checks decided, by box: set (true) or taken off (false).
	// Changed once the batch has read what it needs, so the page's style and layout are
	// worked out once a batch rather than again after every box's mark.
	function mark(marks) {
		marks.forEach(function (tiled, element) {
			if (tiled) {
				element.setAttribute(FLAG, '');
			} else {
				element.removeAttribute(FLAG);
			}
		});
	}

	function unwatch(element, marks) {
		if (watched.has(element)) {
			watched.delete(element);
			resizes.unobserve(element);
		}
		if (element.hasAttribute(FLAG)) {
			marks.set(element, false);
		}
	}

	// Check a watched box's mark. One without an area (hidden, or not laid out) draws no
	// background, so it keeps its mark until it has one: its size changes then.
	function check(element, marks) {
		if (!element.isConnected || !(element.offsetWidth > 0 && element.offsetHeight > 0)) {
			return;
		}
		var style = getComputedStyle(element);
		if (style.backgroundImage.indexOf('url(') === -1) {
			unwatch(element, marks);
			return;
		}
		var tiled = tiles(element, style);
		if (tiled !== element.hasAttribute(FLAG)) {
			marks.set(element, tiled);
		}
	}

	// Every check runs here, after the frame's layout and before its paint: the boxes'
	// styles and sizes are worked out already, so reading them costs little.
	var resizes = new ResizeObserver(function (entries) {
		var marks = new Map();
		for (var i = 0; i < entries.length; i++) {
			check(entries[i].target, marks);
		}
		mark(marks);
	});

	// Have `element` checked after the next frame's layout (if it has a size then): a
	// box observed anew is reported once it has one.
	function watch(element) {
		if (watched.has(element)) {
			resizes.unobserve(element);
		} else {
			watched.add(element);
		}
		resizes.observe(element);
	}

	// A box the page changed: watched (and so checked) if a rule may give it a
	// background image.
	function consider(element, marks) {
		if (mayHaveImage(element)) {
			watch(element);
		} else if (watched.has(element) || element.hasAttribute(FLAG)) {
			unwatch(element, marks);
		}
	}

	function considerAll(root, marks) {
		consider(root, marks);
		var all = root.getElementsByTagName('*');
		for (var i = 0; i < all.length; i++) {
			consider(all[i], marks);
		}
	}

	// Whether a box's class changing (from the record's old value to what it is now)
	// can change a background in it or below it: a class some rule with a background
	// image names came or went. Any other change shows as before.
	function classesMatter(record) {
		if (imageKeys.all || imageClasses.anyClass) {
			return true;
		}
		var before = (record.oldValue || '').split(/\s+/).filter(function (name) { return name !== ''; });
		var now = record.target.classList;
		for (var i = 0; i < before.length; i++) {
			if (imageClasses.names.has(before[i]) && !now.contains(before[i])) {
				return true;
			}
		}
		for (var j = 0; j < now.length; j++) {
			if (imageClasses.names.has(now[j]) && before.indexOf(now[j]) === -1) {
				return true;
			}
		}
		return false;
	}

	// Whether one of `roots` holds `element`, which it then covers.
	function inside(element, roots) {
		for (var node = element.parentNode; node !== null; node = node.parentNode) {
			if (roots.has(node)) {
				return true;
			}
		}
		return false;
	}

	// A box taken out of the page stops being watched (the page may put it back).
	function forget(root) {
		if (root.isConnected) {
			return;
		}
		var all = [root].concat(Array.prototype.slice.call(root.getElementsByTagName('*')));
		for (var i = 0; i < all.length; i++) {
			if (watched.has(all[i])) {
				watched.delete(all[i]);
				resizes.unobserve(all[i]);
			}
		}
	}

	// A class can change the background of anything inside; a `style` only the box's own.
	var changes = new MutationObserver(function (records) {
		var roots = new Set();
		var boxes = new Set();
		for (var i = 0; i < records.length; i++) {
			var record = records[i];
			if (record.type === 'childList') {
				for (var a = 0; a < record.addedNodes.length; a++) {
					if (record.addedNodes[a].nodeType === Node.ELEMENT_NODE) {
						roots.add(record.addedNodes[a]);
					}
				}
				for (var r = 0; r < record.removedNodes.length; r++) {
					if (record.removedNodes[r].nodeType === Node.ELEMENT_NODE) {
						forget(record.removedNodes[r]);
					}
				}
			} else if (record.attributeName === 'class') {
				if (classesMatter(record)) {
					roots.add(record.target);
				}
			} else if (
				// A `style` that names no background before or after only changes the
				// box's size or whether it shows, which its size changing reports.
				BACKGROUND_STYLE.test(record.oldValue || '') || BACKGROUND_STYLE.test(record.target.getAttribute('style') || '')) {
				boxes.add(record.target);
			}
		}
		var marks = new Map();
		roots.forEach(function (root) {
			if (root.isConnected && !inside(root, roots)) {
				considerAll(root, marks);
			}
		});
		boxes.forEach(function (element) {
			if (element.isConnected) {
				consider(element, marks);
			}
		});
		mark(marks);
	});

	function considerPage() {
		var marks = new Map();
		considerAll(document.documentElement, marks);
		mark(marks);
	}

	function start() {
		fixSheets();
		changes.observe(document.documentElement, {
			subtree: true,
			childList: true,
			attributes: true,
			attributeOldValue: true,
			attributeFilter: ['class', 'style']
		});
		considerPage();
	}

	// A style sheet that arrives later (mods add theirs) can give any box a background.
	document.addEventListener('load', function (e) {
		if (e.target && (e.target.tagName === 'LINK' || e.target.tagName === 'STYLE') && fixSheets() &&
			document.readyState !== 'loading') {
			considerPage();
		}
	}, true);
	document.addEventListener('DOMContentLoaded', start);
})();
