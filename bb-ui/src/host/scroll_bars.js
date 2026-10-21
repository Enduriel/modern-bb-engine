// Injected into the page after compat.js, before the page's own scripts.
//
// Scroll bars that follow their content when it changes instead of polling it. Each
// aciScrollBar (vanilla's lists, the console, and every mod list built on them) runs
// `setTimeout(update, 200)` for as long as it exists, shown or not. Its update re-reads
// the bar's size, padding and its content's scroll size (forcing style and layout); if
// any changed it lays the bar out again and checks once more 10ms later, and either way
// redraws the thumb at the content's scroll position. Modded, ~50 of them wake the page
// ~250 times a second, ~4% of a core, for bars that rarely change.
// INTENTIONAL DIVERGENCE (docs/differences.md, "UI scroll bars"):
// here the 200ms timer is never armed. The same update runs when something it reads may
// have changed:
// - the bar's element or a child of its content changed size (ResizeObserver): content
//   added or removed, text and images laid out, shown after being hidden, the window;
// - its subtree changed (MutationObserver): elements, text, classes and styles, other
//   than the bar's own parts, which the update itself draws, and attributes changed
//   and changed back at once;
// - its content scrolled other than through the plugin (a script setting `scrollTop`);
// - an image inside it loaded, a font loaded, or a style sheet came or went.
// The thumb then follows within the task, where the original's took up to 200ms. The
// plugin's 10ms re-check after a change is kept. Vanilla's and mods' files are unchanged:
// only the timer the plugin's own update arms is refused, and only once this layer
// watches the bars.
(function () {
	'use strict';

	var nativeSetTimeout = window.setTimeout;
	var POLL_DELAY = 200;
	// The plugin's poll: `if ($this.data('customUi' + ...)) { ...; $this.trigger('update'); }`.
	var POLL_SOURCE = /customUi[\s\S]*trigger\(\s*['"]update['"]\s*\)/;
	// Set once the bars are watched; until then they poll as in the original.
	var watching = false;

	function isPoll(handler) {
		return typeof handler === 'function' && POLL_SOURCE.test(Function.prototype.toString.call(handler));
	}

	window.setTimeout = function setTimeout(handler, delay) {
		if (watching && delay === POLL_DELAY && isPoll(handler)) {
			// The plugin keeps this as its timer's handle: 0 is "none" to it.
			return 0;
		}
		return nativeSetTimeout.apply(window, arguments);
	};

	var NAMESPACE = '.aciScrollBar';
	var BAR_PARTS = '.aciSb_bar_v, .aciSb_bar_h, .aciSb_resize';

	// Bar element → its watchers.
	var bars = new WeakMap();
	// Every watched bar, held weakly, for what concerns them all (fonts, style sheets).
	var all = new Set();
	var pruneAt = 64;

	// Updates asked for since the last run, run together in one task.
	var dirty = new Set();
	var pending = false;

	function schedule(record) {
		dirty.add(record);
		if (!pending) {
			pending = true;
			nativeSetTimeout.call(window, runUpdates, 0);
		}
	}

	function runUpdates() {
		pending = false;
		var records = Array.from(dirty);
		dirty.clear();
		var $ = window.jQuery;
		records.forEach(function (record) {
			if (bars.get(record.element) !== record) {
				return;
			}
			if (!$ || !$(record.element).data('customUi' + NAMESPACE)) {
				unwatch(record);
				return;
			}
			$(record.element).trigger('update');
		});
	}

	function scheduleAll() {
		all.forEach(function (ref) {
			var element = ref.deref();
			var record = element && bars.get(element);
			if (record) {
				schedule(record);
			} else {
				all.delete(ref);
			}
		});
	}

	function prune() {
		all.forEach(function (ref) {
			var element = ref.deref();
			if (!element || !bars.has(element)) {
				all.delete(ref);
			}
		});
		pruneAt = Math.max(64, all.size * 2);
	}

	// Whether a batch of mutations may have changed what the bar reads. Not when all it
	// did was the plugin drawing the bar (its bars and resizer, the size it gives its
	// content's box), or an attribute changed and changed back within the batch: jQuery
	// does that, reading the size of a hidden element (`$.swap` of its style) and
	// searching under one (a passing `id`), as do hover classes on and off at once.
	function mayChange(mutations) {
		var first = new Map();
		for (var i = 0; i < mutations.length; i++) {
			var mutation = mutations[i];
			if (mutation.type !== 'attributes') {
				return true;
			}
			var target = mutation.target;
			if (target.closest(BAR_PARTS) !== null ||
				(mutation.attributeName === 'style' && target.classList.contains('aciSb_cnt'))) {
				continue;
			}
			var seen = first.get(target);
			if (!seen) {
				seen = new Map();
				first.set(target, seen);
			}
			if (!seen.has(mutation.attributeName)) {
				seen.set(mutation.attributeName, mutation.oldValue);
			}
		}
		var changed = false;
		first.forEach(function (attributes, target) {
			attributes.forEach(function (oldValue, name) {
				changed = changed || target.getAttribute(name) !== oldValue;
			});
		});
		return changed;
	}

	function observeChildren(record) {
		var children = record.container.children;
		var seen = new Set();
		for (var i = 0; i < children.length; i++) {
			seen.add(children[i]);
			if (!record.children.has(children[i])) {
				record.resize.observe(children[i], { box: 'border-box' });
			}
		}
		record.children.forEach(function (child) {
			if (!seen.has(child)) {
				record.resize.unobserve(child);
			}
		});
		record.children = seen;
	}

	function watch(element, container) {
		var record = {
			element: element,
			container: container,
			children: new Set(),
			resize: null,
			mutations: null,
			onScroll: null
		};
		record.resize = new ResizeObserver(function () {
			schedule(record);
		});
		record.mutations = new MutationObserver(function (mutations) {
			if (mutations.some(function (m) { return m.type === 'childList' && m.target === record.container; })) {
				observeChildren(record);
			}
			if (mayChange(mutations)) {
				schedule(record);
			}
		});
		record.onScroll = function () {
			schedule(record);
		};
		record.resize.observe(element);
		observeChildren(record);
		record.mutations.observe(element, { childList: true, subtree: true, attributes: true, attributeOldValue: true, characterData: true });
		container.addEventListener('scroll', record.onScroll, { passive: true });
		bars.set(element, record);
		all.add(new WeakRef(element));
		if (all.size >= pruneAt) {
			prune();
		}
		return record;
	}

	function unwatch(record) {
		record.resize.disconnect();
		record.mutations.disconnect();
		record.container.removeEventListener('scroll', record.onScroll);
		dirty.delete(record);
		if (bars.get(record.element) === record) {
			bars.delete(record.element);
		}
	}

	// Watch `element` if it is a bar (again, if set up anew since), stop if no longer.
	function sync($, element) {
		var record = bars.get(element);
		var container = $(element).data('customUi' + NAMESPACE) ? element.querySelector('.aciSb_cnt') : null;
		if (record && record.container !== container) {
			unwatch(record);
			record = null;
		}
		if (!record && container) {
			watch(element, container);
		}
	}

	// Every set-up and teardown goes through the plugin's `_aciScrollBar`, including a
	// later copy of it a mod might install.
	function watchedInit(init, $) {
		if (typeof init !== 'function' || init.bbWatched) {
			return init;
		}
		var watched = function () {
			var result = init.apply(this, arguments);
			for (var i = 0; i < this.length; i++) {
				if (this[i] instanceof Element) {
					sync($, this[i]);
				}
			}
			return result;
		};
		Object.assign(watched, init);
		watched.bbWatched = true;
		return watched;
	}

	// Images inside a bar's content: their load may change its size where their boxes'
	// don't (a box with a fixed size around them).
	function imageLoaded(image) {
		var node = image.parentElement;
		while (node) {
			var content = node.closest('.aciSb_cnt');
			if (!content || !content.parentElement) {
				return;
			}
			var record = bars.get(content.parentElement);
			if (record) {
				schedule(record);
			}
			node = content.parentElement.parentElement;
		}
	}

	function isStyle(node) {
		return node instanceof HTMLStyleElement || node instanceof HTMLLinkElement;
	}

	function touchesStyles(mutation) {
		if (mutation.type === 'characterData') {
			return isStyle(mutation.target.parentNode);
		}
		return isStyle(mutation.target) ||
			Array.prototype.some.call(mutation.addedNodes, isStyle) ||
			Array.prototype.some.call(mutation.removedNodes, isStyle);
	}

	document.addEventListener('load', function (e) {
		var target = e.target;
		if (target instanceof HTMLImageElement) {
			imageLoaded(target);
		} else if (target instanceof HTMLLinkElement || target instanceof HTMLStyleElement) {
			scheduleAll();
		}
	}, true);

	document.addEventListener('DOMContentLoaded', function () {
		var $ = window.jQuery;
		if (!$ || !$.fn || !$.aciScrollBar) {
			return;
		}
		var init = watchedInit($.fn._aciScrollBar, $);
		if (typeof init !== 'function') {
			return;
		}
		Object.defineProperty($.fn, '_aciScrollBar', {
			configurable: true,
			enumerable: true,
			get: function () { return init; },
			set: function (value) { init = watchedInit(value, $); }
		});
		// Style sheets added, removed or rewritten (not the scripts mods insert there).
		new MutationObserver(function (mutations) {
			if (mutations.some(touchesStyles)) {
				scheduleAll();
			}
		}).observe(document.head, { childList: true, subtree: true, characterData: true });
		if (document.fonts) {
			document.fonts.addEventListener('loadingdone', scheduleAll);
		}
		watching = true;
		document.querySelectorAll('.aciScrollBar').forEach(function (element) {
			sync($, element);
		});
	});
})();
