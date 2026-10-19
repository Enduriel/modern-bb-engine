// Injected into the page after bootstrap.js, before the page's own scripts.
//
// The order the page's dynamically inserted scripts run in. Mods' JS loaders (Modern
// Hooks', old mod_hooks', the dev console's) create a `<script src>` per file and append
// them all in one go. Coherent GT ran them like this (probed in the original):
// - In insertion order, whatever each one's size and whether it was loaded before.
//   Chromium runs dynamically inserted scripts as they arrive unless `async` is off, so
//   created scripts get `async = false`, which puts them in one queue in insertion order.
// - Except that a script whose URL an earlier inserted script is still loading runs right
//   after that one (after its load event), ahead of the scripts inserted between them:
//   Coherent loaded the URL once for both. Nested Tooltips registers its `setup.js` with
//   Modern Hooks twice (by name, then again with its whole folder); the second copy
//   resets the mod's namespace, so where it runs decides whether the hooks between keep
//   what they saved there.
// - A script inserted again later, once its URL has loaded, runs again in its place in
//   the queue (the dev console reloads files this way).
//
// So a script is inserted inert (with a type that isn't JavaScript) when an earlier copy
// of its URL is still loading. When that copy has run and fired its load event, its
// source is read again (synchronously, from the page's own files) and run for each
// waiting copy, as the copy, which then fires its own load event. Insertions through
// `appendChild`, `insertBefore` and `replaceChild` are seen, as the old WebKit's mods
// use; the newer `append`/`before`/... (which Coherent lacked) are left alone.
(function () {
	'use strict';
	var createElement = Document.prototype.createElement;
	Document.prototype.createElement = function (name) {
		var element = createElement.apply(this, arguments);
		if (typeof name === 'string' && name.toLowerCase() === 'script') {
			element.async = false;
		}
		return element;
	};

	// URL → the copies inserted while its first copy loads, in insertion order.
	var loading = new Map();
	// Scripts already inserted once: a script runs at most once, wherever it moves.
	var inserted = new WeakSet();

	function isClassicScript(script) {
		var type = script.getAttribute('type');
		if (type === null || type === '') {
			var language = script.getAttribute('language');
			return language === null || language === '' || /^(java|ecma)script/i.test(language);
		}
		return /^\s*(text|application)\/(x-)?(java|ecma)script\s*$/i.test(type);
	}

	// `node`, about to go into `parent`: an external script that starts loading there.
	function startsLoading(node, parent) {
		return node instanceof HTMLScriptElement && !inserted.has(node) && parent.isConnected
			&& node.hasAttribute('src') && node.src !== '' && isClassicScript(node);
	}

	// The text of `url`, as the browser would decode it for a script, or null.
	function readSource(url) {
		try {
			var request = new XMLHttpRequest();
			request.open('GET', url, false);
			request.overrideMimeType('text/javascript; charset=' + document.characterSet);
			request.send();
			var ok = request.status === 200 || (request.status === 0 && request.responseText !== '');
			return ok ? request.responseText : null;
		} catch (e) {
			return null;
		}
	}

	// Run `source` as `script` (a copy of the script it came from): a classic script in
	// the page's global scope, with `document.currentScript` naming the copy.
	function runAs(script, source, url) {
		var body = createElement.call(document, 'script');
		body.text = source + '\n//# sourceURL=' + url;
		inserted.add(body);
		Object.defineProperty(document, 'currentScript', {
			configurable: true,
			get: function () { return script; }
		});
		try {
			(document.head || document.documentElement).appendChild(body);
		} finally {
			delete document.currentScript;
			if (body.parentNode) {
				body.parentNode.removeChild(body);
			}
		}
	}

	function loaded(url, waiting, ok) {
		if (loading.get(url) === waiting) {
			loading.delete(url);
		}
		var source = ok && waiting.length ? readSource(url) : null;
		waiting.forEach(function (script) {
			if (source === null) {
				script.dispatchEvent(new Event('error'));
				return;
			}
			runAs(script, source, url);
			script.dispatchEvent(new Event('load'));
		});
	}

	// Insert `node` into `parent` with `insert()`, applying the rules above.
	function inserting(node, parent, insert) {
		if (!startsLoading(node, parent)) {
			return insert();
		}
		inserted.add(node);
		var url = node.src;
		var waiting = loading.get(url);
		if (waiting) {
			// Inert while inserted: the type isn't read again once it's in.
			var type = node.getAttribute('type');
			node.setAttribute('type', 'text/x-bb-waits-for-its-first-copy');
			var result;
			try {
				result = insert();
			} finally {
				if (type === null) {
					node.removeAttribute('type');
				} else {
					node.setAttribute('type', type);
				}
			}
			waiting.push(node);
			return result;
		}
		waiting = [];
		loading.set(url, waiting);
		node.addEventListener('load', function () { loaded(url, waiting, true); });
		node.addEventListener('error', function () { loaded(url, waiting, false); });
		return insert();
	}

	var appendChild = Node.prototype.appendChild;
	var insertBefore = Node.prototype.insertBefore;
	var replaceChild = Node.prototype.replaceChild;
	Node.prototype.appendChild = function (node) {
		var self = this, args = arguments;
		return inserting(node, this, function () { return appendChild.apply(self, args); });
	};
	Node.prototype.insertBefore = function (node) {
		var self = this, args = arguments;
		return inserting(node, this, function () { return insertBefore.apply(self, args); });
	};
	Node.prototype.replaceChild = function (node) {
		var self = this, args = arguments;
		return inserting(node, this, function () { return replaceChild.apply(self, args); });
	};
})();
