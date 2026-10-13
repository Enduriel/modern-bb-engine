//! Loading the game's sprite sheets from the mounted archives at startup.
//!
//! The original loads every brush under `brushes/` before any script runs, in the
//! order of [`brush_load_order`]. Each file is read from the last archive that has it
//! (a mod's copy replaces vanilla's), like scripts. A file that fails to parse is
//! skipped; the original logs those as "Not a brush sheet definition" or "Brush sheet
//! layout version incompatible" and carries on.

use bb_assets::{BrushSheet, SpriteRegistry, brush_load_order};

use crate::vfs::DatArchive;

/// The loaded sprites, and one message per brush file that couldn't be loaded.
pub struct SpriteLoad {
	pub registry: SpriteRegistry,
	pub failures: Vec<String>,
}

/// Load every brush from `archives` (in load order: a later archive overrides an
/// earlier one).
pub fn load_sprites(archives: &mut [DatArchive]) -> SpriteLoad {
	let names: Vec<String> = archives.iter().flat_map(DatArchive::names).collect();
	let mut registry = SpriteRegistry::new();
	let mut failures = Vec::new();
	for path in brush_load_order(names.iter().map(String::as_str)) {
		let Some(bytes) = archives.iter_mut().rev().find_map(|archive| archive.read(&path)) else {
			failures.push(format!("{path}: unreadable"));
			continue;
		};
		match BrushSheet::parse(&bytes) {
			Ok(sheet) => {
				registry.add_sheet(path, sheet);
			}
			Err(err) => failures.push(format!("{path}: {err}")),
		}
	}
	SpriteLoad { registry, failures }
}
