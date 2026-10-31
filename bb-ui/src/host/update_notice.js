// The engine's update notice: a newer release of the engine is out. Not the game's UI,
// and nothing of it changes the game's or its mods' code: an element of the engine's own
// over the whole page. It is built as MSU builds its update popup, from vanilla's own
// pieces, so it looks like the game's dialogs (and a UI reskin mod styles it with them):
// a `ui-control dialog` on vanilla's dialog background, a title in vanilla's font
// classes, the notes in vanilla's scroll list (`createList`, with vanilla's scroll bar)
// and vanilla's buttons (`createTextButton`). Every rule of its own is scoped under its
// root class (`bb-engine-update-notice`), so none reaches the game. Should vanilla's
// helpers be missing, it falls back to plain elements rather than fail.
//
// It is modal: a dark translucent backdrop covers the page and takes the mouse while it
// shows (the game keeps the keys from the states meanwhile, `bb-app`'s front).
//
// Called as `(op, data)`:
// - `('show', { version, current, notes, link })` shows it (replacing one showing);
// - `('hide')` takes it down;
// - `('shown')` returns whether it shows;
// - `('choose', choice)` chooses as its button does (the player's Escape and Enter, which
//   the game hands it), returning whether it showed.
//
// Its three buttons tell the game through its own handler, `engine.call('bb.updateNotice',
// choice[, version])` (`bb-ui/src/bridge.rs`), then take the notice down:
// - "Open link" opens `link` through the page's `openURL`, as the game's own links do
//   (the game checks it there), choice `open`;
// - "Continue", choice `continue`;
// - "Ignore this version", choice `ignore` with the version.
//
// The release notes are Markdown, written by whoever made the release: they are shown
// as text only (jQuery's `text`), a line at a time, with a tiny subset read from them:
// `#` headings in the title colour, `-`/`*` list items with a bullet, `**`/`__`/backticks
// dropped. No HTML from them ever reaches the page.
(function (op, data) {
	'use strict';
	var ROOT = 'bb-engine-update-notice';
	var old = document.querySelector('.' + ROOT);
	if (op === 'shown') {
		return old !== null;
	}
	if (op === 'choose') {
		var button = old && old.querySelector('[data-choice="' + String(data) + '"]');
		if (!button) {
			return false;
		}
		button.click();
		return true;
	}
	if (old !== null) {
		take(old);
	}
	if (op !== 'show') {
		return true;
	}

	var vanilla = typeof $ === 'function' && $.fn && typeof $.fn.createTextButton === 'function' &&
		typeof $.fn.createList === 'function' && typeof $.fn.findListScrollContainer === 'function';

	// Its own rules, once, each under the root class.
	if (!document.getElementById(ROOT + '-style')) {
		var style = document.createElement('style');
		style.id = ROOT + '-style';
		var r = '.' + ROOT;
		style.textContent = [
			r + ' { position: fixed; left: 0; top: 0; width: 100%; height: 100%; z-index: 2147483000; ',
			'  background: rgba(0, 0, 0, 0.6); }',
			// Vanilla's dialog is absolute, centred by its margins: here at its background's size.
			r + ' > .ui-control.dialog { width: 80.0rem; height: 72.0rem; background-size: 100% 100%; }',
			r + ' > .ui-control.dialog > .header { height: 9.0rem; }',
			r + ' > .ui-control.dialog > .header .title.has-sub-title { top: 1.4rem; }',
			r + ' > .ui-control.dialog > .header .sub-title { top: 5.6rem; height: 2.4rem; line-height: 2.4rem; }',
			r + ' > .ui-control.dialog > .content { top: 11.0rem; bottom: 8.0rem; left: 4.5rem; right: 4.5rem; }',
			r + ' .notes { padding: 0 2.4rem 0 0.6rem; }',
			r + ' .notes > div { width: 100%; word-wrap: break-word; white-space: pre-wrap; line-height: 1.35; }',
			r + ' .notes > .heading { margin-top: 0.8rem; }',
			r + ' .notes > .item { padding-left: 1.6rem; text-indent: -1.2rem; }',
			r + ' > .ui-control.dialog > .footer { bottom: 2.4rem; height: 4.3rem; display: flex; justify-content: center; }',
			r + ' > .ui-control.dialog > .footer > .notice-button { width: 17.5rem; height: 4.3rem; margin: 0 0.8rem; }',
			// Without vanilla's helpers: plain elements, still in its colours.
			r + ' .plain-list { position: absolute; left: 0; top: 0; width: 100%; height: 100%; overflow-y: auto; }',
			r + ' .plain-button { box-sizing: border-box; border: 0.1rem solid #7c6338; background: #2e2416; color: #dda21f; ',
			'  text-align: center; line-height: 4.1rem; cursor: pointer; }'
		].join('\n');
		document.head.appendChild(style);
	}

	function take(root) {
		if (vanillaList(root)) {
			$(root).find('.notes-list').destroyList();
		}
		if (root.parentNode) {
			root.parentNode.removeChild(root);
		}
	}

	function vanillaList(root) {
		return typeof $ === 'function' && $.fn && typeof $.fn.destroyList === 'function' &&
			root.querySelector('.notes-list.ui-control') !== null;
	}

	function element(tag, className, text) {
		var e = document.createElement(tag);
		if (className) {
			e.className = className;
		}
		if (text !== undefined) {
			e.textContent = text;
		}
		return e;
	}

	// One line of the notes as text, without Markdown's emphasis and code marks.
	function plain(line) {
		return line.replace(/\*\*|__|`/g, '');
	}

	function notes(text) {
		var box = element('div', 'notes');
		var lines = String(text || '').replace(/\r\n?/g, '\n').split('\n');
		var any = false;
		lines.forEach(function (line) {
			var heading = /^\s{0,3}#{1,6}\s+(.*)$/.exec(line);
			var item = /^\s*[-*+]\s+(.*)$/.exec(line);
			if (heading) {
				box.appendChild(element('div', 'heading title-font-normal font-bold font-color-title', plain(heading[1])));
			} else if (item) {
				box.appendChild(element('div', 'item description-font-normal font-color-description', '• ' + plain(item[1])));
			} else {
				box.appendChild(element('div', 'line description-font-normal font-color-description', plain(line) || ' '));
			}
			any = any || line.trim() !== '';
		});
		if (!any) {
			box.textContent = '';
			box.appendChild(element('div', 'line description-font-normal font-color-description', 'No release notes.'));
		}
		return box;
	}

	var root = element('div', ROOT);
	var dialog = element('div', 'ui-control dialog dialog-800-720');
	root.appendChild(dialog);
	var header = element('div', 'header');
	var titles = element('div', 'text-container');
	header.appendChild(titles);
	titles.appendChild(element('div', 'title has-sub-title title-font-very-big font-bold font-bottom-shadow font-color-title',
		'A new version is available'));
	var versions = element('div', 'sub-title versions description-font-big font-color-description');
	versions.appendChild(document.createTextNode('Version '));
	versions.appendChild(element('span', 'found font-color-title', String(data.version)));
	versions.appendChild(document.createTextNode(' is out. You have version '));
	versions.appendChild(element('span', 'current font-color-title', String(data.current)));
	versions.appendChild(document.createTextNode('.'));
	titles.appendChild(versions);
	dialog.appendChild(header);
	var content = element('div', 'content');
	dialog.appendChild(content);
	// The page's own handlers never see the notice's mouse events; buttons see them first.
	['mousedown', 'mouseup', 'click', 'dblclick', 'contextmenu', 'wheel', 'mousemove'].forEach(function (type) {
		root.addEventListener(type, function (event) {
			event.stopPropagation();
		});
	});
	// On the page before the list: vanilla's scroll bar measures it as it is made.
	(document.body || document.documentElement).appendChild(root);

	if (vanilla) {
		var list = $(content).createList(8, 'notes-list');
		var scroll = list.findListScrollContainer();
		(scroll || list).append(notes(data.notes));
	} else {
		var plainList = element('div', 'notes-list plain-list');
		plainList.appendChild(notes(data.notes));
		content.appendChild(plainList);
	}

	function choose(choice) {
		take(root);
		if (choice === 'open') {
			engine.call('openURL', String(data.link));
		}
		if (choice === 'ignore') {
			engine.call('bb.updateNotice', choice, String(data.version));
		} else {
			engine.call('bb.updateNotice', choice);
		}
	}

	var footer = element('div', 'footer');
	dialog.appendChild(footer);
	[['open', 'Open link'], ['continue', 'Continue'], ['ignore', 'Ignore this version']].forEach(function (b) {
		var button;
		if (vanilla) {
			// The labels are fixed text: `createTextButton` puts them in as markup.
			button = $(footer).createTextButton(b[1], function () { choose(b[0]); }, 'notice-button', 1)[0];
		} else {
			button = element('div', 'notice-button plain-button text-font-normal', b[1]);
			button.addEventListener('click', function () { choose(b[0]); });
			footer.appendChild(button);
		}
		button.setAttribute('data-choice', b[0]);
	});
	return true;
})
