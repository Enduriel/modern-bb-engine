// Vanilla's FPS module (`ui/screens/root/modules/fps_module.js`) on the root screen,
// shown and fed by the game. Vanilla creates the module but leaves its registration
// commented out in `RootScreen.registerModules` and `unregisterModules`
// (`root_screen.js`), and the original never shows it. This does what those lines
// would, only once the player turns the display on: the page stays vanilla's until
// then.
//
// Called as `(op, value)`:
// - `('show', shown)` turns the display on or off. It's remembered on the page, so the
//   module shows as soon as there is a root screen to show it on.
// - `('set', text)` keeps `text` as the value and returns whether the module the game's
//   `callFromSQ("RootScreen", "FPSModule", "setFPS", ...)` reaches has its label: the
//   game sends that call only then (vanilla's `setFPS` throws without one).
//
// The module is looked up at each call, as `callFromSQ` does: `Screens.RootScreen`
// may not be the one there was. Modern Hooks registers a second, never-connected
// `RootScreen` over the first once the mods' JS has loaded; the first one's container
// stays on the page and the second has none. A root screen can also lose its container
// (`hide`, `onDisconnection`) and get a new one (`show`, `onConnection`). So the module
// `callFromSQ` reaches is registered into its root screen's container (moved there once
// it has one), or else into the page's root-screen element, as soon as either exists,
// and a module left on another root screen, or in a container no longer on the page,
// is taken down. Each root screen seen gets its `registerModules`/`unregisterModules`
// wrapped, so a container it makes again gets the module again, with the last value.
//
// One change to its look while it shows: vanilla's box is 8rem wide, room for a
// two-digit rate in its monospace font (its markup says 60), so at 100 fps and more
// the value wrapped onto a second line, below the box. The box grows to fit instead.
(function (op, value) {
	'use strict';
	var state = window.__bbFPS;
	if (!state) {
		var style = document.createElement('style');
		style.setAttribute('data-bb-compat', 'fps');
		style.textContent = '.root-screen .fps-container { width: auto; min-width: 8.0rem; ' +
			'padding-right: 1.0rem; white-space: nowrap; }';
		state = window.__bbFPS = { shown: false, value: '', style: style, module: null, sync: null };
	}

	function moduleOf(root) {
		if (!root || typeof root.getModule !== 'function') {
			return null;
		}
		var module = root.getModule('FPSModule');
		return module && typeof module.register === 'function' ? module : null;
	}

	function onPage(element) {
		return element !== null && element.length > 0 && document.body.contains(element[0]);
	}

	function takeDown(module) {
		if (module !== null && module.mContainer !== null) {
			module.unregister();
		}
	}

	function wrap(root) {
		if (root.__bbFPSWrapped) {
			return;
		}
		root.__bbFPSWrapped = true;
		var register = root.registerModules;
		var unregister = root.unregisterModules;
		root.registerModules = function () {
			var result = register.apply(this, arguments);
			state.sync();
			return result;
		};
		root.unregisterModules = function () {
			// Before vanilla's: its `destroyDIV` empties the container under the module.
			takeDown(moduleOf(this));
			if (state.module !== null && state.module.mContainer === null) {
				state.module = null;
			}
			return unregister.apply(this, arguments);
		};
	}

	// Brings the page in line with `state`; returns whether the module `callFromSQ`
	// reaches has its label.
	state.sync = function () {
		var root = typeof Screens === 'object' && Screens !== null ? Screens['RootScreen'] : null;
		var module = moduleOf(root);
		if (module !== null) {
			wrap(root);
		}
		if (state.module !== null && state.module !== module) {
			takeDown(state.module);
			state.module = null;
		}
		if (module === null) {
			return false;
		}
		var own = typeof root.getContainer === 'function' ? root.getContainer() : null;
		// Gone from the page with its container, or in another root screen's while its
		// own has one now.
		if (module.mContainer !== null &&
			(!onPage(module.mContainer) || (onPage(own) && !$.contains(own[0], module.mContainer[0])))) {
			module.unregister();
		}
		if (!state.shown) {
			takeDown(module);
			state.module = null;
			return false;
		}
		if (module.mContainer === null) {
			var container = onPage(own) ? own : $('.root-screen').first();
			if (!onPage(container)) {
				return false;
			}
			module.register(container);
			if (module.mFPSLabel === null) {
				return false;
			}
			module.setFPS(state.value);
		}
		state.module = module;
		return module.mFPSLabel !== null;
	};

	if (op === 'show') {
		state.shown = value;
		if (value && !state.style.parentNode) {
			document.head.appendChild(state.style);
		} else if (!value && state.style.parentNode) {
			state.style.parentNode.removeChild(state.style);
		}
		state.sync();
		return true;
	}
	state.value = value;
	return state.sync();
})
