//! Scripts the engine itself ships, as if a mod did: files of the game's `scripts/` that
//! no archive holds, listed by `IO.enumerateFiles` and loaded by `include` like the rest.
//! They come before the game's files: a mod's file at the same path doesn't replace them.
//!
//! - `scripts/!mods_preload/mod_modern_engine`: registers this engine with Modern Hooks
//!   as the mod `mod_modern_engine`, at the release version, so a mod requires the engine
//!   as it requires any mod (`mod.require("mod_modern_engine >= 0.4.0")`), and in the
//!   original game, where nothing registers it, Modern Hooks' popup names it as missing.
//!   preload.nut's sweep of `scripts/` runs it with the other mods' preload files, after
//!   Modern Hooks' `scripts/config/` made `::Hooks` and before Modern Hooks' last preload
//!   file (`~~modern_hooks`) checks the requirements. Without Modern Hooks it does
//!   nothing. For mod authors: `docs/modding.md`.

use crate::source::LoadedScript;

/// The mod id this engine registers with Modern Hooks.
pub const MOD_ID: &str = "mod_modern_engine";

/// The version it registers: the release's (every workspace crate shares it).
pub const MOD_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The registration script's path.
const REGISTRATION: &str = "scripts/!mods_preload/mod_modern_engine";

fn registration() -> String {
	format!(
		r#"// The modern engine, registered with Modern Hooks as a mod, so mods can require it.
if ("Hooks" in ::getroottable() && "register" in ::Hooks && !::Hooks.hasMod("{MOD_ID}"))
	::Hooks.register("{MOD_ID}", "{MOD_VERSION}", "Modern Engine");
"#
	)
}

/// The engine's script at `path` (normalized, without extension), if it ships one.
pub(crate) fn load(path: &str) -> Option<LoadedScript> {
	(path == REGISTRATION).then(|| LoadedScript::Source(registration().into_bytes()))
}

/// Whether the engine ships a script at `path`.
pub(crate) fn has(path: &str) -> bool {
	path == REGISTRATION
}

/// The engine's scripts under `prefix`, as `ScriptSource::list_prefix` matches one: at a
/// path-segment boundary, with or without the trailing `/`.
pub(crate) fn listed_under(prefix: &str) -> impl Iterator<Item = &'static str> {
	let dir = if prefix.is_empty() || prefix.ends_with('/') {
		prefix.to_string()
	} else {
		format!("{prefix}/")
	};
	[REGISTRATION].into_iter().filter(move |path| path.starts_with(&dir))
}

/// A source's listing of `prefix` with the engine's scripts there merged in, in order.
pub(crate) fn merge_listing(prefix: &str, listed: Option<Vec<String>>) -> Option<Vec<String>> {
	let mut ours = listed_under(prefix).peekable();
	if ours.peek().is_none() {
		return listed;
	}
	let mut paths = listed.unwrap_or_default();
	paths.extend(ours.map(str::to_string));
	paths.sort();
	paths.dedup();
	Some(paths)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn listed_only_under_its_folders() {
		for prefix in ["", "scripts", "scripts/", "scripts/!mods_preload", "scripts/!mods_preload/"] {
			assert_eq!(listed_under(prefix).collect::<Vec<_>>(), [REGISTRATION], "{prefix:?}");
		}
		for prefix in ["scripts/dlc/", "scripts/!mods", "script", "scripts/!mods_preload/mod_modern_engine"] {
			assert_eq!(listed_under(prefix).count(), 0, "{prefix:?}");
		}
	}

	#[test]
	fn merged_in_order_without_duplicates() {
		let listed = Some(vec![
			"scripts/!mods_preload/mod_a".to_string(),
			"scripts/!mods_preload/mod_modern_engine".to_string(),
			"scripts/!mods_preload/~~modern_hooks".to_string(),
		]);
		assert_eq!(merge_listing("scripts/!mods_preload", listed.clone()), listed);
		assert_eq!(merge_listing("scripts/dlc/", None), None, "a missing folder stays missing");
		assert_eq!(merge_listing("scripts/", None), Some(vec![REGISTRATION.to_string()]));
	}
}
