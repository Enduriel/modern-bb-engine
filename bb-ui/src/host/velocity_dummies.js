// Injected into the page after teardown.js, before the page's own scripts.
//
// Velocity's measuring elements, let go once measured. To convert between units (px, %,
// em) for an animation, the UI's Velocity (1.2.3, `ui/extern/velocity.min.js`) makes a
// `div` of its own, registers it with `Velocity.init` (which keeps a record for it in
// jQuery's data store), adds it beside the animated element, reads its size and takes
// it out again with the DOM's own `removeChild`. jQuery forgets an element's data only
// when jQuery itself removes it (`.remove()`, `.empty()`, `.html()`), so every such
// element stays in jQuery's store for good, with its record and the computed style
// Velocity read from it: ~130 elements a battle with the AI fighting both sides, more
// with a player's turns (measured). They are
// small, but each is made in the middle of a battle's churn and holds on to the Blink
// heap's pages around it, so the page's memory grew battle after battle.
//
// The original runs the same scripts and keeps them too, so this is a deliberate
// divergence (docs/differences.md, "Velocity's measuring elements"). Nothing can tell: the
// element is Velocity's own, reachable only through jQuery's store, and Velocity never
// uses its record again (the next measurement makes a new element).
//
// Here an element Velocity registers while it is in no document, with nothing in it and
// no attributes, is looked at again once the current task's work is done: if it is
// still in no document and is styled as Velocity's measuring element (100em of left
// padding, content-box sizing, hidden overflow), jQuery forgets it, as if it had been
// removed through jQuery.
(function () {
	'use strict';

	var waiting = [];
	var wrappers = [];

	function isMeasuring(element) {
		var style = element.style;
		return !element.isConnected && style && style.paddingLeft === '100em' &&
			style.boxSizing === 'content-box' && style.overflow === 'hidden';
	}

	function forget() {
		var elements = waiting;
		waiting = [];
		var gone = elements.filter(isMeasuring);
		if (gone.length) {
			window.jQuery.cleanData(gone);
		}
	}

	function wrap(velocity) {
		if (!velocity || typeof velocity.init !== 'function' || wrappers.indexOf(velocity.init) >= 0) {
			return;
		}
		var init = velocity.init;
		var wrapped = function (element) {
			var result = init.apply(this, arguments);
			if (element && element.nodeType === 1 && element.parentNode === null &&
				element.attributes.length === 0 && element.firstChild === null) {
				if (!waiting.length) {
					queueMicrotask(forget);
				}
				waiting.push(element);
			}
			return result;
		};
		wrappers.push(wrapped);
		velocity.init = wrapped;
	}

	document.addEventListener('DOMContentLoaded', function () {
		var $ = window.jQuery;
		if (!$ || typeof $.cleanData !== 'function') {
			return;
		}
		wrap($.Velocity);
		wrap(window.Velocity);
	});
})();
