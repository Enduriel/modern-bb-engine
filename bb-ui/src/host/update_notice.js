// The engine's update notice: a newer release of the engine is out. Not the game's UI
// and nothing of it changes the game's or its mods' code: one element of the engine's
// own over the whole page, its content in a shadow root, so neither the game's CSS nor a
// mod's reaches it, and it reaches nothing of theirs. It is modal: its backdrop covers
// the page and takes the mouse while it shows (the game keeps the keys from the states
// meanwhile, `bb-app`'s front).
//
// Called as `(op, data)`:
// - `('show', { version, current, notes, link })` shows it (replacing one showing);
// - `('hide')` takes it down;
// - `('shown')` returns whether it shows.
//
// Its three buttons tell the game through its own handler, `engine.call('bb.updateNotice',
// choice[, version])` (`bb-ui/src/bridge.rs`), then take the notice down:
// - "Open link" opens `link` through the page's `openURL`, as the game's own links do
//   (the game checks it there), choice `open`;
// - "Continue", choice `continue`;
// - "Ignore this version", choice `ignore` with the version.
//
// The release notes are Markdown, written by whoever made the release: they are shown
// as text only (`textContent`), a line at a time, with a tiny subset read from them:
// `#` headings in bold, `-`/`*` list items with a bullet, `**`/`__`/backticks dropped.
// No HTML from them ever reaches the page.
(function (op, data) {
	'use strict';
	var TAG = 'bb-engine-update-notice';
	var old = document.querySelector(TAG);
	if (op === 'shown') {
		return old !== null;
	}
	if (old !== null) {
		old.parentNode.removeChild(old);
	}
	if (op !== 'show') {
		return true;
	}

	var host = document.createElement(TAG);
	// Its own box over everything, whatever the page's CSS says of unknown elements.
	host.setAttribute('style', 'all: initial !important; position: fixed !important; left: 0 !important; ' +
		'top: 0 !important; width: 100% !important; height: 100% !important; display: block !important; ' +
		'z-index: 2147483647 !important; pointer-events: auto !important;');
	var root = host.attachShadow({ mode: 'open' });

	var style = document.createElement('style');
	style.textContent = [
		':host { all: initial; }',
		'.backdrop { position: absolute; left: 0; top: 0; right: 0; bottom: 0; background: rgba(0, 0, 0, 0.66); ',
		'  display: flex; align-items: center; justify-content: center; cursor: default; user-select: none; }',
		'.box { box-sizing: border-box; width: 56rem; max-width: 92%; max-height: 86%; display: flex; flex-direction: column; ',
		'  background: linear-gradient(#2b2117, #1a140e); border: 0.2rem solid #6b5532; ',
		'  box-shadow: 0 0 0 0.1rem #000, 0 0.6rem 2.4rem rgba(0, 0, 0, 0.8); padding: 2.0rem 2.4rem; color: #c8b89a; ',
		'  font-family: "FreeUniversal-Regular", sans-serif; font-size: 1.6rem; line-height: 1.35; }',
		'.title { font-family: "Cinzel-Bold", serif; font-size: 2.4rem; color: #dda21f; text-align: center; margin-bottom: 1.0rem; }',
		'.versions { text-align: center; margin-bottom: 1.4rem; }',
		'.versions b { color: #ffffff; font-weight: normal; }',
		'.notes { flex: 1 1 auto; overflow-y: auto; min-height: 4rem; background: rgba(0, 0, 0, 0.35); ',
		'  border: 0.1rem solid #4a3a22; padding: 1.0rem 1.2rem; margin-bottom: 1.6rem; user-select: text; white-space: pre-wrap; ',
		'  word-break: break-word; }',
		'.notes .heading { color: #e6d6b4; font-weight: bold; margin-top: 0.6rem; }',
		'.notes .item { padding-left: 1.4rem; text-indent: -1.0rem; }',
		'.notes .empty { font-style: italic; color: #8a7a5e; }',
		'.buttons { display: flex; justify-content: center; gap: 1.2rem; flex-wrap: wrap; }',
		'button { font-family: "Cinzel-Bold", serif; font-size: 1.6rem; color: #e6d6b4; background: linear-gradient(#4a3a24, #2e2416); ',
		'  border: 0.15rem solid #7c6338; padding: 0.6rem 1.6rem; min-width: 14rem; cursor: pointer; outline: none; }',
		'button:hover { color: #ffffff; border-color: #c49a4a; background: linear-gradient(#5a4628, #3a2d1a); }',
		'button:active { background: #2a2014; }'
	].join('\n');
	root.appendChild(style);

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
				box.appendChild(element('div', 'heading', plain(heading[1])));
			} else if (item) {
				box.appendChild(element('div', 'item', '• ' + plain(item[1])));
			} else {
				box.appendChild(element('div', '', plain(line) || ' '));
			}
			any = any || line.trim() !== '';
		});
		if (!any) {
			box.textContent = '';
			box.appendChild(element('div', 'empty', 'No release notes.'));
		}
		return box;
	}

	var backdrop = element('div', 'backdrop');
	var box = element('div', 'box');
	box.setAttribute('role', 'dialog');
	box.appendChild(element('div', 'title', 'A new version is available'));
	var versions = element('div', 'versions');
	versions.appendChild(document.createTextNode('Version '));
	versions.appendChild(element('b', 'found', String(data.version)));
	versions.appendChild(document.createTextNode(' is out. You have version '));
	versions.appendChild(element('b', 'current', String(data.current)));
	versions.appendChild(document.createTextNode('.'));
	box.appendChild(versions);
	box.appendChild(notes(data.notes));

	function choose(choice) {
		if (host.parentNode) {
			host.parentNode.removeChild(host);
		}
		if (choice === 'open') {
			engine.call('openURL', String(data.link));
		}
		if (choice === 'ignore') {
			engine.call('bb.updateNotice', choice, String(data.version));
		} else {
			engine.call('bb.updateNotice', choice);
		}
	}

	var buttons = element('div', 'buttons');
	[['open', 'Open link'], ['continue', 'Continue'], ['ignore', 'Ignore this version']].forEach(function (b) {
		var button = element('button', b[0], b[1]);
		button.setAttribute('data-choice', b[0]);
		button.addEventListener('click', function (event) {
			event.stopPropagation();
			choose(b[0]);
		});
		buttons.appendChild(button);
	});
	box.appendChild(buttons);
	backdrop.appendChild(box);
	root.appendChild(backdrop);
	// The page's own handlers never see the notice's mouse events.
	['mousedown', 'mouseup', 'click', 'dblclick', 'contextmenu', 'wheel', 'mousemove'].forEach(function (type) {
		host.addEventListener(type, function (event) {
			event.stopPropagation();
		});
	});
	(document.body || document.documentElement).appendChild(host);
	return true;
})
