// Injected into the page first, before bootstrap.js and the page's own scripts.
//
// What the page has to do, for the game to tell whether it may rest: stay parked
// across the game's frames, with no turn and no frame begun (host/page_rest.rs).
// bootstrap.js sends `__bbWork()` with each park:
//
//   {"busy": reason | null, "timer": ms | null, "keys": count}
//
// `busy` names why the page needs its next turn: "ran" (one of its own tasks ran since
// it last parked: a timer, an animation frame, an event; what it changed may still need
// a frame), "changed" (its document changed since it last said: elements, text,
// attributes and styles, or a scroll position set; e.g. by the game's calls into it,
// which a frame must show), "frames" (it waits for an animation frame: Velocity's and jQuery's ticks),
// "editing" (a text field has the focus: its caret blinks), "fonts" (fonts are
// loading), "animations" (a CSS animation or transition runs). `timer` is how long
// until its next timer is due. The page's timers and animation frames are wrapped to
// keep that count; what they do is unchanged. `keys` counts the keydown and keyup events
// it has seen (also `window.__bbKeysSeen`): the game hands a frame's keys over and
// waits until the page has seen them (host/session_input.rs).
(function () {
	'use strict';

	var nativeSetTimeout = window.setTimeout;
	var nativeClearTimeout = window.clearTimeout;
	var nativeSetInterval = window.setInterval;
	var nativeClearInterval = window.clearInterval;
	var nativeRequestFrame = window.requestAnimationFrame;
	var nativeCancelFrame = window.cancelAnimationFrame;
	var slice = Array.prototype.slice;

	// Timer handle (timeouts and intervals share them) -> when it's next due
	// (performance.now()).
	var timers = new Map();
	// Animation frames asked for and not yet run.
	var frames = new Set();
	// Whether one of the page's own tasks ran since it last parked.
	var ran = false;

	// A timer's delay as the browser takes it: anything not above 0 is 0, and one that
	// doesn't fit 32 bits is 1 (HTML's timer steps; nesting's 4ms minimum only makes it
	// later, which an early turn covers).
	function delayOf(delay) {
		var ms = Number(delay);
		if (!(ms > 0)) {
			return 0;
		}
		return ms > 2147483647 ? 1 : ms;
	}

	// Run a timer's handler as the browser does: a function with the extra arguments and
	// `this` the window, or a string as code in the global scope.
	function invoke(handler, args) {
		if (typeof handler === 'function') {
			return handler.apply(window, args);
		}
		return (0, eval)(String(handler));
	}

	window.setTimeout = function setTimeout(handler, delay) {
		var args = slice.call(arguments, 2);
		var handle = nativeSetTimeout.call(window, function () {
			timers.delete(handle);
			ran = true;
			return invoke(handler, args);
		}, delay);
		timers.set(handle, performance.now() + delayOf(delay));
		return handle;
	};

	window.setInterval = function setInterval(handler, delay) {
		var args = slice.call(arguments, 2);
		var every = delayOf(delay);
		var handle = nativeSetInterval.call(window, function () {
			// Before the handler, which may clear it.
			timers.set(handle, performance.now() + every);
			ran = true;
			return invoke(handler, args);
		}, delay);
		timers.set(handle, performance.now() + every);
		return handle;
	};

	window.clearTimeout = function clearTimeout(handle) {
		timers.delete(handle);
		return nativeClearTimeout.call(window, handle);
	};

	window.clearInterval = function clearInterval(handle) {
		timers.delete(handle);
		return nativeClearInterval.call(window, handle);
	};

	window.requestAnimationFrame = function requestAnimationFrame(callback) {
		var handle = nativeRequestFrame.call(window, function (time) {
			frames.delete(handle);
			ran = true;
			return callback(time);
		});
		frames.add(handle);
		return handle;
	};

	window.cancelAnimationFrame = function cancelAnimationFrame(handle) {
		frames.delete(handle);
		return nativeCancelFrame.call(window, handle);
	};

	// Old libraries take the prefixed names first.
	if ('webkitRequestAnimationFrame' in window) {
		window.webkitRequestAnimationFrame = window.requestAnimationFrame;
	}
	['webkitCancelAnimationFrame', 'webkitCancelRequestAnimationFrame'].forEach(function (name) {
		if (name in window) {
			window[name] = window.cancelAnimationFrame;
		}
	});

	// Events are the page's tasks too: input, loads (pictures, scripts, fonts), scrolls,
	// messages, and animations starting or ending. Seen first, changed nothing.
	function mark() {
		ran = true;
	}
	[
		'load', 'error', 'scroll', 'message',
		'mousemove', 'mousedown', 'mouseup', 'click', 'dblclick', 'contextmenu', 'wheel',
		'input', 'change', 'focusin', 'focusout',
		'animationstart', 'animationiteration', 'animationend', 'transitionrun', 'transitionend'
	].forEach(function (type) {
		window.addEventListener(type, mark, { capture: true, passive: true });
	});

	// A key's events, which the game hands over every frame while it's held (as the
	// original's queue does), are a task of the page's like the others, except where
	// what the browser does with them by default shows nothing: a key that types, or a
	// function or modifier key, outside a form control, with no shortcut modifier held
	// (Ctrl+A selects all). For those Chromium neither scrolls, edits, activates nor
	// moves the focus; what the page's own listeners change counts as ever (the
	// document's changes, timers, animation frames). So holding W to scroll the map, or
	// a modifier, doesn't keep a page with nothing to do from resting.
	var keys = 0;
	function control(target) {
		return !!target && (target.isContentEditable || /^(INPUT|TEXTAREA|SELECT|BUTTON)$/.test(target.tagName || ''));
	}
	function quietKey(e) {
		if (control(e.target)) {
			return false;
		}
		var code = e.keyCode;
		// The modifier keys themselves (their own events carry their flag).
		if (code === 16 || code === 17 || code === 18) {
			return true;
		}
		if (e.ctrlKey || e.altKey || e.metaKey) {
			return false;
		}
		if (e.type === 'keypress') {
			// What it typed: Space scrolls and Enter activates on their keypress.
			return e.charCode > 32;
		}
		// Digits, letters, the function keys (those Coherent has no code for as -1,
		// which Chromium holds as 65535).
		return (code >= 48 && code <= 57) || (code >= 65 && code <= 90) || (code >= 112 && code <= 135) || code === -1 || code === 65535;
	}
	['keydown', 'keyup', 'keypress'].forEach(function (type) {
		window.addEventListener(type, function (e) {
			if (type !== 'keypress') {
				keys++;
			}
			if (!quietKey(e)) {
				ran = true;
			}
		}, { capture: true, passive: true });
	});
	Object.defineProperty(window, '__bbKeysSeen', {
		get: function () {
			return keys;
		},
		enumerable: false
	});

	// Changes to the document since the page last said what it has to do: taken as
	// they're counted, or, once a task ends, delivered here.
	var changes = new MutationObserver(function () {
		ran = true;
	});
	changes.observe(document, { subtree: true, childList: true, attributes: true, characterData: true });
	// Scroll positions set by script aren't changes to the document, but show.
	['scrollTop', 'scrollLeft'].forEach(function (name) {
		var property = Object.getOwnPropertyDescriptor(Element.prototype, name);
		if (property && property.set) {
			Object.defineProperty(Element.prototype, name, {
				configurable: true,
				enumerable: property.enumerable,
				get: property.get,
				set: function (value) {
					ran = true;
					property.set.call(this, value);
				}
			});
		}
	});
	['scrollTo', 'scrollBy'].forEach(function (name) {
		var scroll = window[name];
		window[name] = function () {
			ran = true;
			return scroll.apply(window, arguments);
		};
	});

	// Whether a focused field shows a caret.
	function editing() {
		var element = document.activeElement;
		if (!element) {
			return false;
		}
		if (element.isContentEditable || element.tagName === 'TEXTAREA') {
			return true;
		}
		return element.tagName === 'INPUT' && /^(text|search|password|email|number|tel|url|)$/i.test(element.type || '');
	}

	// Whether a CSS animation or transition (or a script's Web Animation) runs or is
	// about to start.
	function animating() {
		if (typeof document.getAnimations !== 'function') {
			return false;
		}
		var animations = document.getAnimations();
		for (var i = 0; i < animations.length; ++i) {
			if (animations[i].pending || animations[i].playState === 'running') {
				return true;
			}
		}
		return false;
	}

	function busy() {
		if (ran) {
			return 'ran';
		}
		if (changes.takeRecords().length > 0) {
			return 'changed';
		}
		if (frames.size > 0) {
			return 'frames';
		}
		if (editing()) {
			return 'editing';
		}
		if (document.fonts && document.fonts.status === 'loading') {
			return 'fonts';
		}
		return animating() ? 'animations' : null;
	}

	Object.defineProperty(window, '__bbWork', {
		value: function () {
			var reason = busy();
			ran = false;
			changes.takeRecords();
			var now = performance.now();
			var next = Infinity;
			timers.forEach(function (due) {
				if (due < next) {
					next = due;
				}
			});
			return JSON.stringify({ busy: reason, timer: next === Infinity ? null : Math.max(0, next - now), keys: keys });
		},
		enumerable: false
	});
})();
