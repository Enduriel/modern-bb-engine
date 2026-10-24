// Injected into the page before its own scripts run.
//
// It plays the native half of Coherent GT: `window.engine` exists before
// `ui/coherent.js` runs, so coherent.js takes its "attached" path and builds the rest
// of the API (on/off/call/_Result/...) on top of the few members defined here:
//
//   engine.SendMessage(name, id, args...)  every `engine.call`, run in the game
//   engine.TriggerEvent(name, args...)     JS-side `engine.trigger`; the game has no
//                                          listeners, so this is a no-op
//   engine.BindingsReady()                 coherent.js asks for `_OnReady`
//
// Calls run inline both ways, as in the original. An `engine.call` blocks (a `call`
// request through the renderer's native __bbRequest, host/bridge_client.rs) until the
// game has run it. If the game calls into the page meanwhile, the answer is
// {"nested": entry}: the page runs the entry and reports back in a `resume` request
// (what it has to do now, page_work.js, a newline, then the entry's result), then keeps
// waiting. The final answer
// is {"done": true, ...}. A `callSQ` result reaches its promise in a later task
// (`_Result`), as in the original; the other calls are never answered (their promises
// stay pending, also as in the original), and coherent.js's table of calls waiting for
// an answer (`engine._ActiveRequests`) lets them go. A call made by one of the page's own tasks
// (an animation frame, a timer, input), rather than from inside one of the game's
// calls, is a `task` request instead, so the game can tell the two apart.
//
// While the game runs its scripts, the page is parked: it waits the same way in a
// `park` request that the game answers when it's done, and the game's calls
// arrive as that request's answers. The page's own tasks then run only between the
// game's scripts, as in the original, whose UI runs on the game's thread. A park says
// what the page has to do (page_work.js), so the game can leave a page with nothing
// to do parked across its frames (host/page_rest.rs).
//
// The game reaches the page with:
//   __bbCall(entryJson)  run one entry now, returning its result as JSON
//   __bbEval(code)       evaluate test code, returning {"ok": value} or {"error": text}
//   __bbPark(result)     one step of parking until the game lets the page go on
// An entry is ["t", event, args...]: engine._trigger(event, args...), or ["e", code]:
// evaluate test code, as __bbEval does.
//
// Copy and paste go through the game's clipboard (the system's, in the windowed game;
// `bb-ui/src/clipboard.rs`), as Coherent's went through Windows': the text a copy or
// cut takes (Ctrl+C/X, execCommand('copy'/'cut')) is handed to the game, and a paste
// (Ctrl+V, execCommand('paste')) inserts the game's text. `navigator.clipboard`'s text
// calls do the same.
(function () {
	'use strict';
	// The renderer's native call into the game (`host/bridge_client.rs`), taken off
	// `window` before any page script runs.
	var bridgeRequest = window.__bbRequest;
	delete window.__bbRequest;

	function reportError(what, e) {
		console.error(what + ': ' + (e && e.stack ? e.stack : e));
	}

	// One blocking call into the game. Throws if the game can't be reached.
	function request(kind, payload) {
		return JSON.parse(bridgeRequest(kind, payload === undefined ? '' : String(payload)));
	}

	// A JS value as the original hands it to a script (probed): `undefined` is null,
	// functions are empty arrays, objects keep their own enumerable properties, and
	// NaN/Infinity become -2147483648 (numbers are truncated to integers game-side).
	function toWire(value) {
		if (value === undefined || value === null) {
			return null;
		}
		if (typeof value === 'number') {
			return isFinite(value) ? value : -2147483648;
		}
		if (typeof value === 'function') {
			return [];
		}
		if (Array.isArray(value)) {
			var array = [];
			for (var i = 0; i < value.length; ++i) {
				array.push(toWire(value[i]));
			}
			return array;
		}
		if (typeof value === 'object') {
			var object = {};
			Object.keys(value).forEach(function (key) {
				object[key] = toWire(value[key]);
			});
			return object;
		}
		return value;
	}

	// The game's entries running now, nested ones included.
	var running = 0;

	function run(entry) {
		var engine = window.engine;
		running++;
		try {
			if (entry[0] === 't') {
				return engine._trigger.apply(engine, entry.slice(1));
			}
			if (entry[0] === 'e') {
				return evaluate(entry[1]);
			}
		} finally {
			running--;
		}
	}

	function evaluate(code) {
		try {
			var value = (0, eval)(code);
			return { ok: value === undefined ? null : value };
		} catch (e) {
			return { error: String(e && e.stack ? e.stack : e) };
		}
	}

	function toJson(result) {
		try {
			return JSON.stringify(result === undefined ? null : result);
		} catch (e) {
			return 'null';
		}
	}

	// A game call into the page that throws goes to coherent.js's `engine._ThrowError`,
	// as Coherent's does: an Error is logged (its name, message and stack, through
	// console.error) and anything else thrown, a string say, fails in there and logs
	// nothing (probed 2026-10-21). The script's call gets null either way.
	function runReporting(entry) {
		try {
			return run(entry);
		} catch (e) {
			var engine = window.engine;
			if (engine && typeof engine._ThrowError === 'function') {
				try {
					engine._ThrowError(e);
				} catch (ignored) {
				}
			} else {
				reportError(entry.slice(1, 5).join('.'), e);
			}
		}
	}

	// What the page has to do (page_work.js): sent with each park, and after each of
	// the game's calls into a waiting page, so the game can leave a page with nothing to
	// do parked across its frames (host/page_rest.rs).
	var work = window.__bbWork;

	function workNow() {
		return typeof work === 'function' ? work() : '';
	}

	// Wait in request `kind` until the game answers it, running the game's calls into
	// the page that arrive meanwhile.
	function wait(kind, payload) {
		var reply = request(kind, payload);
		while (reply.nested !== undefined) {
			var result = toJson(runReporting(reply.nested));
			reply = request('resume', workNow() + '\n' + result);
		}
		return reply;
	}

	function answer(id, reply) {
		var engine = window.engine;
		if (reply.result !== undefined) {
			setTimeout(function () { engine._Result(id, reply.result); }, 0);
		} else if (reply.noValue) {
			setTimeout(function () { engine._Result(id); }, 0);
		} else if (reply.errors) {
			setTimeout(function () { engine._OnError(id, reply.errors); }, 0);
		} else if (engine._ActiveRequests) {
			// A void call (`asyncCallSQ`, `registerScreen`, …): never answered, so its
			// promise stays pending, but coherent.js's table of calls waiting for an
			// answer lets it go, or it would keep every one: ~400 a battle, each a
			// promise and its emitter. Nothing else holds it: SQ.call drops it.
			delete engine._ActiveRequests[id];
		}
	}

	var engine = {
		SendMessage: function () {
			var args = Array.prototype.slice.call(arguments);
			var reply;
			try {
				reply = wait(running ? 'call' : 'task', JSON.stringify(toWire(args)));
			} catch (e) {
				reportError('engine.call(' + args[0] + ')', e);
				return;
			}
			answer(args[1], reply);
		},
		TriggerEvent: function () {},
		BindingsReady: function () {
			engine._OnReady();
		},
		RemoveOnHandler: function () {},
		__observeLifetime: function () {},
		translate: function (text) {
			return text;
		}
	};
	window.engine = engine;

	function define(name, value) {
		Object.defineProperty(window, name, { value: value, enumerable: false });
	}

	define('__bbCall', function (entryJson) {
		return toJson(runReporting(JSON.parse(entryJson)));
	});

	define('__bbEval', function (code) {
		return toJson(run(['e', code]));
	});

	// The text a copy or cut takes: what the page put on the event's clipboardData
	// if it took the copy over, else the selection (never a password field's).
	function copiedText(event) {
		if (event.defaultPrevented) {
			return event.clipboardData ? event.clipboardData.getData('text/plain') : '';
		}
		var element = document.activeElement;
		if (element && (element.tagName === 'TEXTAREA' || element.tagName === 'INPUT')) {
			if (element.type === 'password') {
				return '';
			}
			try {
				if (typeof element.selectionStart === 'number') {
					return element.value.substring(element.selectionStart, element.selectionEnd);
				}
			} catch (e) {
				// No selection on this kind of input (type=number, ...).
			}
		}
		var selection = window.getSelection();
		return selection ? String(selection) : '';
	}

	function writeClipboard(text) {
		try {
			request('clipboard-write', text);
		} catch (e) {
			reportError('copying', e);
		}
	}

	// The game's clipboard text, or null when it has none.
	function readClipboard() {
		try {
			var text = request('clipboard-read');
			return typeof text === 'string' ? text : null;
		} catch (e) {
			reportError('pasting', e);
			return null;
		}
	}

	// After the page's own listeners, so a copy the page took over is seen as it left it.
	function onCopy(event) {
		var text = copiedText(event);
		if (text !== '') {
			writeClipboard(text);
		}
	}
	window.addEventListener('copy', onCopy, false);
	window.addEventListener('cut', onCopy, false);
	// Put `text` where the caret is in `element`, over its selection, as a paste
	// does: no more of it than the field's `maxlength` leaves room for (vanilla's name
	// fields have one). Not through execCommand('insertText'): inside
	// execCommand('paste') a second command doesn't run.
	function insertText(element, text) {
		if (element && (element.tagName === 'TEXTAREA' || element.tagName === 'INPUT')) {
			try {
				var start = element.selectionStart, end = element.selectionEnd;
				if (element.maxLength >= 0) {
					var room = Math.max(0, element.maxLength - (element.value.length - (end - start)));
					// Not half a surrogate pair.
					var code = text.charCodeAt(room - 1);
					text = text.slice(0, code >= 0xd800 && code <= 0xdbff ? room - 1 : room);
					if (text.length === 0) {
						return;
					}
				}
				element.setRangeText(text, start, end, 'end');
			} catch (e) {
				return; // No caret in this kind of input (type=number, ...).
			}
			element.dispatchEvent(new InputEvent('input', { bubbles: true, inputType: 'insertFromPaste', data: text }));
			return;
		}
		var selection = window.getSelection();
		if (!selection || selection.rangeCount === 0) {
			return;
		}
		var range = selection.getRangeAt(0);
		range.deleteContents();
		var node = document.createTextNode(text);
		range.insertNode(node);
		range.setStartAfter(node);
		range.collapse(true);
		selection.removeAllRanges();
		selection.addRange(range);
	}

	// First, so the page's own listeners see the paste as handled.
	window.addEventListener('paste', function (event) {
		var text = readClipboard();
		if (text === null) {
			return;
		}
		event.preventDefault();
		// The page's own listeners read the game's text, as Coherent's saw Windows'
		// (the event's own clipboardData is Chromium's internal clipboard).
		try {
			var data = new DataTransfer();
			data.setData('text/plain', text);
			Object.defineProperty(event, 'clipboardData', { value: data });
		} catch (e) {
		}
		var target = document.activeElement;
		if (target && (target.isContentEditable || target.tagName === 'TEXTAREA' || target.tagName === 'INPUT')) {
			insertText(target, text);
		}
	}, true);
	if (navigator.clipboard) {
		navigator.clipboard.writeText = function (text) {
			writeClipboard(String(text));
			return Promise.resolve();
		};
		navigator.clipboard.readText = function () {
			var text = readClipboard();
			return Promise.resolve(text === null ? '' : text);
		};
	}

	// One step of a park (renderer.rs runs the loop): with no argument, park; with the
	// result of the game's last call into the parked page, report it. Returns the next
	// call the game sends (an entry, as JSON, for `__bbCall`), or '' once the game lets
	// the page go on. The renderer runs each call on its own, not nested in this
	// function, so the promise callbacks it queues run as it returns, before the page
	// says what it has to do (as after a call into a page that isn't parked).
	define('__bbPark', function (result) {
		var reply;
		try {
			reply = result ? request('resume', workNow() + '\n' + result) : request('park', workNow());
		} catch (e) {
			reportError('parking', e);
			return '';
		}
		if (reply.nested === undefined) {
			turn = reply.turn || null;
			return '';
		}
		return JSON.stringify(reply.nested);
	});

	// The turn the game gave the page as it let it go on (a frame's, after its scripts,
	// or one while it waits on the page), if it gave one: the renderer calls this once a
	// park ends, outside it, and the page's held timers run (page_work.js).
	var turn = null;
	var advance = window.__bbAdvance;
	define('__bbTurn', function () {
		var given = turn;
		turn = null;
		if (given !== null && typeof advance === 'function') {
			advance(given);
		}
		return '';
	});
})();
