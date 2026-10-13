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
	var BUILT_IN_CSS =
		':where(input, select, textarea, button) { font: inherit; }\n' +
		':where(input[type=checkbox], input[type=radio]) { margin: 0; font: 16px "Times New Roman"; }\n' +
		':where(input[type=range]) { margin: 18px 0; color: #909090; }\n';
	document.addEventListener('DOMContentLoaded', function () {
		var style = document.createElement('style');
		style.setAttribute('data-bb-compat', '');
		style.textContent = BUILT_IN_CSS;
		document.head.insertBefore(style, document.head.firstChild);
	});

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
