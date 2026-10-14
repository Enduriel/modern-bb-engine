// Injected into the page after compat.js, before the page's own scripts.
//
// Images stay loaded once loaded, by URL, as in Coherent's image cache. Chromium keeps
// a loaded image only while something on the page uses it: when the game takes its
// screens down (a return to the main menu, the end of a battle) and builds them again,
// every picture is fetched and decoded anew, as a new image, while the old decodes stay
// in the compositor's cache until memory runs short. A world screen's worth of
// pictures piled up on every return to the menu that way, where the original's memory
// stays flat (probed). Keeping one image per URL lets the new screens reuse the
// loaded picture and its decode. Procedural images (portraits) are drawn per entity,
// under URLs that don't come back, so they aren't kept.
(function () {
	'use strict';

	var kept = new Map();
	document.addEventListener('load', function (e) {
		var image = e.target;
		if (!(image instanceof HTMLImageElement)) {
			return;
		}
		var url = image.currentSrc;
		if (url.indexOf('coui://') !== 0 || url.indexOf('coui://procedural/') === 0 || kept.has(url)) {
			return;
		}
		var keep = new Image();
		keep.src = url;
		kept.set(url, keep);
	}, true);
})();
