//! Which `.brush` files the original loads at startup, and in what order.
//!
//! At startup the engine loads the directory `brushes/` (`0x49a621` → `0x5254f0`).
//! For a directory it calls `PHYSFS_enumerateFiles`, which lists each name once
//! across all mounted archives, sorted by `strcmp`, and skips `desktop.ini`
//! (`0x51ec50`). Then, entry by entry, a subdirectory is walked the same way, and a
//! file is loaded when the text after its last `.` is exactly `brush`
//! (`0x4a17d0`). Which archive's copy of a file is read is the file system's usual
//! overlay rule; this module only decides the names and their order.
//!
//! The order is per directory, so it isn't a plain sort of the full paths:
//! `brushes/a/x.brush` loads before `brushes/a.brush`, because the entry `a` sorts
//! before `a.brush`, while a full-path sort puts `.` before `/`.

use std::collections::BTreeSet;

/// The directory the original loads brushes from.
pub const BRUSH_DIR: &str = "brushes/";

/// The brush files to load, in the original's order, given the path of every file
/// in the mounted archives (any order, duplicates allowed). Paths ending in `/` are
/// directory entries and only mark a directory.
pub fn brush_load_order<'a>(paths: impl IntoIterator<Item = &'a str>) -> Vec<String> {
	let under: BTreeSet<&str> = paths
		.into_iter()
		.filter_map(|path| path.strip_prefix(BRUSH_DIR))
		.collect();
	let mut order = Vec::new();
	walk(&under, "", &mut order);
	order
}

/// Load `dir` (relative to [`BRUSH_DIR`], empty or ending in `/`) entry by entry.
fn walk(paths: &BTreeSet<&str>, dir: &str, order: &mut Vec<String>) {
	// Each name directly in `dir`, and whether anything lies below it.
	let mut entries: BTreeSet<(&str, bool)> = BTreeSet::new();
	for rest in paths.iter().filter_map(|path| path.strip_prefix(dir)) {
		match rest.split_once('/') {
			Some((name, _)) => entries.insert((name, true)),
			None => entries.insert((rest, false)),
		};
	}
	let mut names: Vec<(&str, bool)> = Vec::new();
	for (name, is_dir) in entries {
		if name.is_empty() {
			continue;
		}
		// A name present as both: walk it as a directory.
		match names.last_mut() {
			Some(last) if last.0 == name => last.1 |= is_dir,
			_ => names.push((name, is_dir)),
		}
	}
	for (name, is_dir) in names {
		if name == "desktop.ini" {
			continue;
		}
		if is_dir {
			walk(paths, &format!("{dir}{name}/"), order);
		} else if extension(name) == "brush" {
			order.push(format!("{BRUSH_DIR}{dir}{name}"));
		}
	}
}

/// The text after the last `.`, or empty without one.
fn extension(name: &str) -> &str {
	name.rfind('.').map_or("", |dot| &name[dot + 1..])
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn lists_brush_files_sorted_and_once() {
		let paths = [
			"brushes/world_detail.brush",
			"brushes/msu_world.brush",
			"brushes/entity_0.brush",
			"brushes/msu_world.brush",
			"gfx/entity_0.png",
		];
		assert_eq!(
			brush_load_order(paths),
			[
				"brushes/entity_0.brush",
				"brushes/msu_world.brush",
				"brushes/world_detail.brush"
			]
		);
	}

	#[test]
	fn walks_subdirectories_in_entry_order() {
		let paths = [
			"brushes/a.brush",
			"brushes/a/x.brush",
			"brushes/b/c/y.brush",
			"brushes/ab.brush",
		];
		assert_eq!(
			brush_load_order(paths),
			[
				"brushes/a/x.brush",
				"brushes/a.brush",
				"brushes/ab.brush",
				"brushes/b/c/y.brush"
			]
		);
	}

	#[test]
	fn sorts_by_bytes_so_capitals_come_first() {
		let paths = ["brushes/b.brush", "brushes/Z.brush", "brushes/a.brush"];
		assert_eq!(
			brush_load_order(paths),
			["brushes/Z.brush", "brushes/a.brush", "brushes/b.brush"]
		);
	}

	#[test]
	fn needs_exactly_the_brush_extension() {
		let paths = [
			"brushes/upper.BRUSH",
			"brushes/brush",
			"brushes/notes.txt",
			"brushes/two.dots.brush",
			"brushes/x.brushes",
		];
		assert_eq!(brush_load_order(paths), ["brushes/two.dots.brush"]);
	}

	#[test]
	fn skips_desktop_ini_and_directory_entries() {
		let paths = [
			"brushes/",
			"brushes/desktop.ini",
			"brushes/empty/",
			"brushes/a.brush",
		];
		assert_eq!(brush_load_order(paths), ["brushes/a.brush"]);
	}

	#[test]
	fn ignores_files_outside_the_brush_directory() {
		let paths = ["gfx/a.brush", "mod/brushes/a.brush", "brushesx/a.brush"];
		assert!(brush_load_order(paths).is_empty());
	}
}
