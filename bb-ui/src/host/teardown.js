// Injected into the page after image_cache.js, before the page's own scripts.
//
// Handlers that outlive what they were bound for. Three plugins the game's UI uses bind
// handlers on elements other than the one they set up:
// - aciScrollBar, on `html`, to follow a drag of the bar;
// - createImage, on `window`, to refit its image when the window resizes;
// - iCheck, on every label for its checkbox's id, anywhere in the page (the options
//   panels of the main menu and of the in-game menu use the same ids).
// The plugin's own destroy unbinds the first and third, but several screens never call
// it, and nothing unbinds the second. Each handler holds its element, and through it the
// element's whole removed DOM: ~114 handlers and ~350 nodes, about a megabyte of the
// page's memory, on every return to the main menu. The original keeps the same handlers
// (the JS is vanilla's) and its memory stays flat anyway, so this is a deliberate
// divergence.
//
// Here a handler such a plugin binds elsewhere belongs to the element it set up, and
// is unbound when jQuery cleans that element up (`.remove()`, `.empty()`, `.html()`),
// as the element's own handlers are. It could only have acted on the removed element.
// A page can tell only by counting the handlers jQuery lists.
(function () {
	'use strict';

	// Element → the handlers it owns on other elements, as [element, types, handler,
	// selector].
	var owned = new WeakMap();
	// The handlers bound by each tracked plugin call in progress, innermost last.
	var binding = [];

	function adopt(owner, bound) {
		var elsewhere = bound.filter(function (b) {
			return b[0] !== owner && !(b[0] instanceof Node && owner.contains(b[0]));
		});
		if (elsewhere.length) {
			owned.set(owner, (owned.get(owner) || []).concat(elsewhere));
		}
	}

	// Track plugin `name`: what it binds elsewhere belongs to the element
	// `ownerOf(set, result)` names, if any.
	function track($, name, ownerOf) {
		var plugin = $.fn[name];
		if (typeof plugin !== 'function') {
			return;
		}
		var tracked = function () {
			var bound = [];
			binding.push(bound);
			var result;
			try {
				result = plugin.apply(this, arguments);
			} finally {
				binding.pop();
			}
			var owner = bound.length ? ownerOf(this, result) : null;
			if (owner instanceof Node) {
				adopt(owner, bound);
			}
			return result;
		};
		Object.assign(tracked, plugin);
		$.fn[name] = tracked;
	}

	// The element a set holds, if just one: a call on several can't say which of them
	// a handler is for.
	function single(set) {
		return set.length === 1 ? set[0] : null;
	}

	document.addEventListener('DOMContentLoaded', function () {
		var $ = window.jQuery;
		if (!$ || !$.event || typeof $.cleanData !== 'function') {
			return;
		}
		var add = $.event.add;
		$.event.add = function (element, types, handler, data, selector) {
			// Only functions: unbinding `false` would unbind every handler of the type.
			var fn = typeof handler === 'function' ? handler : handler && handler.handler;
			if (binding.length && typeof fn === 'function') {
				binding[binding.length - 1].push([element, types, fn, selector]);
			}
			return add.apply(this, arguments);
		};
		var cleanData = $.cleanData;
		$.cleanData = function (elements) {
			for (var i = 0; i < elements.length; i++) {
				var bound = owned.get(elements[i]);
				if (bound) {
					owned.delete(elements[i]);
					bound.forEach(function (b) {
						$.event.remove(b[0], b[1], b[2], b[3]);
					});
				}
			}
			return cleanData.apply(this, arguments);
		};
		track($, 'aciScrollBar', single);
		track($, 'iCheck', single);
		track($, 'createImage', function (set, image) {
			return image && image[0];
		});
	});
})();
