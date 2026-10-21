//! What a player points at, found in the page: each function returns JS evaluating to
//! the centre `[x, y]` (view pixels) of an element the player can see, or `null`.
//!
//! "Can see" means displayed, with an area, and centred inside the view: screens slide
//! in from outside it, and a player can't click a button that hasn't arrived yet. And
//! not covered there: what the page hit-tests at the centre is the element, inside it,
//! or one of its ancestors (a label that lets the pointer through to its button), not
//! something else on top (a screen fading over it), which a click there would hit
//! instead.

use crate::json::Json;

/// `(function (arg) { body })(arg)`, with `arg` passed as a JSON string, and
/// `centre(el)` defined for the body: `el`'s centre if the player can see it, else
/// `null`.
fn call(body: &str, arg: &str) -> String {
	format!(
		"(function (arg) {{
	function centre(el) {{
		if (el.offsetParent === null) return null;
		var r = el.getBoundingClientRect();
		var x = Math.round(r.left + r.width / 2), y = Math.round(r.top + r.height / 2);
		var seen = r.width > 0 && r.height > 0 && x >= 0 && y >= 0 && x < innerWidth && y < innerHeight;
		if (!seen) return null;
		var hit = document.elementFromPoint(x, y);
		return hit && (el.contains(hit) || hit.contains(el)) ? [x, y] : null;
	}}
	{body}
}})({})",
		Json::Str(arg.to_string()).to_text()
	)
}

/// The `.ui-control` (button) whose text is `label`.
pub(super) fn button(label: &str) -> String {
	call(
		"var all = document.querySelectorAll('.ui-control');
	for (var i = 0; i < all.length; ++i) {
		var point = all[i].textContent.trim() === arg && centre(all[i]);
		if (point) return point;
	}
	return null;",
		label,
	)
}

/// The innermost element whose text is `text` (a list entry, a label, a tab…).
pub(super) fn text(text: &str) -> String {
	call(
		"var all = document.querySelectorAll('body *');
	for (var i = all.length - 1; i >= 0; --i) {
		var point = all[i].textContent.trim() === arg && centre(all[i]);
		if (point) return point;
	}
	return null;",
		text,
	)
}

/// The first element matching the CSS `selector`.
pub(super) fn selector(selector: &str) -> String {
	call(
		"var all = document.querySelectorAll(arg);
	for (var i = 0; i < all.length; ++i) {
		var point = centre(all[i]);
		if (point) return point;
	}
	return null;",
		selector,
	)
}

/// The finders for a target as the `*_when_ready` methods take it, in the order to
/// try them, each with what it looks for (for errors): `css:SELECTOR`, or else a
/// button's label, then any element's text.
pub(super) fn finders(target: &str) -> Vec<(String, String)> {
	match target.strip_prefix("css:") {
		Some(css) => vec![(selector(css), format!("element matching {css:?}"))],
		None => vec![
			(button(target), format!("button labelled {target:?}")),
			(text(target), format!("element with the text {target:?}")),
		],
	}
}
