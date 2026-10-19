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
// {"nested": entry}: the page runs the entry and reports back in a `resume` request,
// then keeps waiting. The final answer
// is {"done": true, ...}. A `callSQ` result reaches its promise in a later task
// (`_Result`), as in the original; the other calls are never answered (their promises
// stay pending, also as in the original). A call made by one of the page's own tasks
// (an animation frame, a timer, input), rather than from inside one of the game's
// calls, is a `task` request instead, so the game can tell the two apart.
//
// While the game runs its scripts, the page is parked: it waits the same way in a
// `park` request that the game answers when it's done, and the game's calls
// arrive as that request's answers. The page's own tasks then run only between the
// game's scripts, as in the original, whose UI runs on the game's thread.
//
// The game reaches the page with:
//   __bbCall(entryJson)  run one entry now, returning its result as JSON
//   __bbEval(code)       evaluate test code, returning {"ok": value} or {"error": text}
//   __bbPark()           park until the game lets the page go on
// An entry is ["t", event, args...]: engine._trigger(event, args...), or ["e", code]:
// evaluate test code, as __bbEval does.
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

	function runReporting(entry) {
		try {
			return run(entry);
		} catch (e) {
			reportError(entry.slice(1, 5).join('.'), e);
		}
	}

	// Wait in request `kind` until the game answers it, running the game's calls into
	// the page that arrive meanwhile.
	function wait(kind, payload) {
		var reply = request(kind, payload);
		while (reply.nested !== undefined) {
			reply = request('resume', toJson(runReporting(reply.nested)));
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

	// Coherent ran scripts that mods insert at runtime in insertion order (probed);
	// Chromium runs dynamically inserted scripts as they arrive unless `async` is off.
	var createElement = Document.prototype.createElement;
	Document.prototype.createElement = function (name) {
		var element = createElement.apply(this, arguments);
		if (typeof name === 'string' && name.toLowerCase() === 'script') {
			element.async = false;
		}
		return element;
	};

	function define(name, value) {
		Object.defineProperty(window, name, { value: value, enumerable: false });
	}

	define('__bbCall', function (entryJson) {
		return toJson(runReporting(JSON.parse(entryJson)));
	});

	define('__bbEval', function (code) {
		return toJson(run(['e', code]));
	});

	define('__bbPark', function () {
		try {
			wait('park', '');
		} catch (e) {
			reportError('parking', e);
		}
		return 'unparked';
	});
})();
