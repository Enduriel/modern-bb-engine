//! Loading the game's sprite sheets from the mounted archives at startup.
//!
//! The original loads every brush under `brushes/` before any script runs, in the
//! order of [`brush_load_order`]. Each file is read from the last archive that has it
//! (a mod's copy replaces vanilla's), like scripts. A file that fails to parse is
//! skipped; the original logs those as "Not a brush sheet definition" or "Brush sheet
//! layout version incompatible" and carries on.

use bb_assets::{BrushSheet, SpriteRegistry, brush_load_order, decode_png};
use squirrel_bb::FogStamp;

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

/// The brush the original stamps into its fog-of-war mask.
pub const FOG_STAMP_BRUSH: &str = "fog_of_war_mask";

/// The fog-of-war stamp: [`FOG_STAMP_BRUSH`]'s first frame and its atlas's alpha.
pub fn load_fog_stamp(archives: &mut [DatArchive], registry: &SpriteRegistry) -> Result<FogStamp, String> {
	let (sheet, sprite) = registry.get(FOG_STAMP_BRUSH).ok_or("no fog_of_war_mask brush")?;
	let frame = sprite.frames.first().ok_or("fog_of_war_mask has no frame")?;
	let path = &sheet.sheet.texture;
	let bytes = archives
		.iter_mut()
		.rev()
		.find_map(|archive| archive.read(path))
		.ok_or_else(|| format!("{path}: unreadable"))?;
	let image = decode_png(&bytes).map_err(|e| format!("{path}: {e}"))?;
	let [left, right, top, bottom] = frame.rect;
	let (dx, dy) = (f32::from(sprite.offset_x), f32::from(sprite.offset_y));
	Ok(FogStamp {
		alpha: image.rgba.chunks_exact(4).map(|px| px[3]).collect(),
		width: image.width,
		height: image.height,
		uv: frame.uv,
		// A frame's `top` is its low y edge (see `BrushFrame`).
		rect: [left + dx, right + dx, top + dy, bottom + dy],
	})
}
