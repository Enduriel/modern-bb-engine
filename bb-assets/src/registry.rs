//! [`SpriteRegistry`]: every loaded sprite, found by name.
//!
//! The original keeps one `std::map` from sprite hash to sprite for all sheets
//! (global `0x66593c`, map at `+0xc`). The sheet loader inserts each sprite with an
//! `emplace`, so when a hash is already present the new sprite is dropped: **the first
//! sprite loaded under a name wins**, both within one file and across files. Files
//! load in the order of [`brush_load_order`](crate::brush_load_order), which makes
//! this observable with mods. MSU's `brushes/msu_world.brush` sorts before vanilla's
//! `world_detail.brush` and `world_decals.brush` and redefines three of their sprites,
//! so MSU's versions are the ones drawn.
//!
//! `doesBrushExist(name)` is `false` for an empty name and otherwise a lookup of
//! the name's [`name_hash`] (`0x505820` → `0x504a10`).

use std::collections::HashMap;

use crate::brush::{BrushSheet, BrushSprite};
use crate::hash::name_hash;

/// Where a registered sprite lives: its sheet's load index and its index in the sheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SpriteId {
	pub sheet: u32,
	pub sprite: u32,
}

/// One loaded `.brush` file.
#[derive(Clone, Debug)]
pub struct LoadedSheet {
	/// The file's path in the game's file system, e.g. `brushes/entity_0.brush`.
	pub path: String,
	pub sheet: BrushSheet,
}

/// All loaded sheets and the sprite lookup over them.
#[derive(Clone, Debug, Default)]
pub struct SpriteRegistry {
	sheets: Vec<LoadedSheet>,
	by_hash: HashMap<u64, SpriteId>,
	shadowed: usize,
}

impl SpriteRegistry {
	pub fn new() -> Self {
		Self::default()
	}

	/// Register a sheet loaded from `path`, after every sheet added before it. A
	/// sprite whose hash is already registered is kept in the sheet but not
	/// reachable by name (the first one wins). Returns how many sprites became
	/// reachable.
	pub fn add_sheet(&mut self, path: impl Into<String>, sheet: BrushSheet) -> usize {
		let sheet_index = self.sheets.len() as u32;
		let mut added = 0;
		for (index, sprite) in sheet.sprites.iter().enumerate() {
			let id = SpriteId {
				sheet: sheet_index,
				sprite: index as u32,
			};
			match self.by_hash.entry(sprite.hash) {
				std::collections::hash_map::Entry::Occupied(_) => self.shadowed += 1,
				std::collections::hash_map::Entry::Vacant(slot) => {
					slot.insert(id);
					added += 1;
				}
			}
		}
		self.sheets.push(LoadedSheet {
			path: path.into(),
			sheet,
		});
		added
	}

	/// `doesBrushExist`: whether a sprite is registered under `name`.
	pub fn contains(&self, name: &str) -> bool {
		self.find(name).is_some()
	}

	/// The sprite registered under `name`. An empty name finds nothing.
	pub fn find(&self, name: &str) -> Option<SpriteId> {
		if name.is_empty() {
			return None;
		}
		self.find_hash(name_hash(name.as_bytes()))
	}

	/// The sprite registered under a stored hash.
	pub fn find_hash(&self, hash: u64) -> Option<SpriteId> {
		self.by_hash.get(&hash).copied()
	}

	/// The sprite behind an id this registry returned.
	pub fn sprite(&self, id: SpriteId) -> &BrushSprite {
		&self.sheets[id.sheet as usize].sheet.sprites[id.sprite as usize]
	}

	/// The sheet holding an id this registry returned.
	pub fn sheet(&self, id: SpriteId) -> &LoadedSheet {
		&self.sheets[id.sheet as usize]
	}

	/// The sprite and its sheet registered under `name`.
	pub fn get(&self, name: &str) -> Option<(&LoadedSheet, &BrushSprite)> {
		let id = self.find(name)?;
		Some((self.sheet(id), self.sprite(id)))
	}

	/// Every sheet, in load order.
	pub fn sheets(&self) -> &[LoadedSheet] {
		&self.sheets
	}

	/// How many sprites are reachable by name.
	pub fn len(&self) -> usize {
		self.by_hash.len()
	}

	/// Whether no sprite is registered.
	pub fn is_empty(&self) -> bool {
		self.by_hash.is_empty()
	}

	/// How many sprites were dropped because an earlier one had the same hash.
	pub fn shadowed(&self) -> usize {
		self.shadowed
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::brush::test_data::{SheetBuilder, sprite};

	fn sheet(texture: &str, sprites: &[BrushSprite]) -> BrushSheet {
		let mut builder = SheetBuilder::new(texture);
		for s in sprites {
			builder = builder.sprite(s.clone());
		}
		BrushSheet::parse(&builder.build()).unwrap()
	}

	#[test]
	fn finds_sprites_by_name_across_sheets() {
		let mut registry = SpriteRegistry::new();
		registry.add_sheet(
			"brushes/a.brush",
			sheet("gfx/a.png", &[sprite("one"), sprite("two")]),
		);
		registry.add_sheet("brushes/b.brush", sheet("gfx/b.png", &[sprite("three")]));

		assert_eq!(registry.len(), 3);
		assert!(registry.contains("two"));
		assert!(!registry.contains("four"));
		let (loaded, found) = registry.get("three").unwrap();
		assert_eq!(loaded.path, "brushes/b.brush");
		assert_eq!(loaded.sheet.texture, "gfx/b.png");
		assert_eq!(found.name, "three");
	}

	#[test]
	fn the_first_sprite_with_a_name_wins_across_files() {
		let mut first = sprite("shared");
		first.width = 1;
		let mut second = sprite("shared");
		second.width = 2;
		let mut registry = SpriteRegistry::new();
		assert_eq!(
			registry.add_sheet("brushes/a.brush", sheet("gfx/a.png", &[first])),
			1
		);
		assert_eq!(
			registry.add_sheet("brushes/b.brush", sheet("gfx/b.png", &[second])),
			0
		);

		assert_eq!(registry.get("shared").unwrap().1.width, 1);
		assert_eq!(registry.shadowed(), 1);
		assert_eq!(
			registry.sheets()[1].sheet.sprites.len(),
			1,
			"the dropped sprite stays in its sheet"
		);
	}

	#[test]
	fn the_first_sprite_with_a_name_wins_within_a_file() {
		let mut first = sprite("dup");
		first.height = 7;
		let mut registry = SpriteRegistry::new();
		registry.add_sheet(
			"brushes/a.brush",
			sheet("gfx/a.png", &[first, sprite("dup")]),
		);
		assert_eq!(registry.get("dup").unwrap().1.height, 7);
		assert_eq!(registry.len(), 1);
	}

	#[test]
	fn lookup_uses_the_stored_hash_not_the_stored_name() {
		let mut renamed = sprite("stored_name");
		renamed.hash = name_hash(b"looked_up");
		let mut registry = SpriteRegistry::new();
		registry.add_sheet("brushes/a.brush", sheet("gfx/a.png", &[renamed]));
		assert!(registry.contains("looked_up"));
		assert!(!registry.contains("stored_name"));
	}

	#[test]
	fn an_empty_name_never_exists() {
		let mut zero = sprite("x");
		zero.hash = 0;
		let mut registry = SpriteRegistry::new();
		registry.add_sheet("brushes/a.brush", sheet("gfx/a.png", &[zero]));
		assert!(registry.find_hash(0).is_some());
		assert!(!registry.contains(""));
	}
}
