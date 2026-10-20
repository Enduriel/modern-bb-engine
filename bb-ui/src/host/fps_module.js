// Shows or hides vanilla's FPS module (`ui/screens/root/modules/fps_module.js`), with
// `shown` given as the call's argument. Vanilla creates the module but leaves its
// registration commented out in `RootScreen.registerModules` and `unregisterModules`
// (`root_screen.js`), and the original never shows it. This does what those lines
// would, run only once the player turns the display on: the page stays vanilla's until
// then.
//
// The root screen's two methods are wrapped on the instance, so a root screen created
// again (connected again) registers the module again while it's shown. Its value comes
// as vanilla meant: `setFPS` through `callFromSQ("RootScreen", "FPSModule", ...)`.
//
// One change to its look while it shows: vanilla's box is 8rem wide, room for a
// two-digit rate in its monospace font (its markup says 60), so at 100 fps and more
// the value wrapped onto a second line, below the box. The box grows to fit instead.
//
// Returns whether the root screen was there to show it on.
(function (shown) {
	'use strict';
	var root = typeof Screens === 'object' && Screens !== null ? Screens['RootScreen'] : null;
	if (!root || !root.mFPSModule) {
		return false;
	}
	var module = root.mFPSModule;
	if (!root.__bbFPS) {
		var style = document.createElement('style');
		style.setAttribute('data-bb-compat', 'fps');
		style.textContent = '.root-screen .fps-container { width: auto; min-width: 8.0rem; ' +
			'padding-right: 1.0rem; white-space: nowrap; }';
		var made = { shown: false, style: style };
		var register = root.registerModules;
		var unregister = root.unregisterModules;
		root.registerModules = function () {
			if (made.shown && module.mContainer === null) {
				module.register(this.mContainer);
			}
			return register.apply(this, arguments);
		};
		root.unregisterModules = function () {
			if (module.mContainer !== null) {
				module.unregister();
			}
			return unregister.apply(this, arguments);
		};
		root.__bbFPS = made;
	}
	var state = root.__bbFPS;
	state.shown = shown;
	if (shown && !state.style.parentNode) {
		document.head.appendChild(state.style);
	} else if (!shown && state.style.parentNode) {
		state.style.parentNode.removeChild(state.style);
	}
	var container = root.getContainer();
	if (shown && container !== null && module.mContainer === null) {
		module.register(container);
	} else if (!shown && module.mContainer !== null) {
		module.unregister();
	}
	return true;
})
