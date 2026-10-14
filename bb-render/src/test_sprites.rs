//! A small sprite registry for the renderers' unit tests: each named sprite is a 2×2
//! picture in its own column of one sheet, so a drawn quad tells which sprite it shows
//! (`name_of`), and an animated one's frames sit side by side within its column
//! (`frame_of`).

use bb_assets::{BrushFrame, BrushSheet, BrushSprite, SheetHeader, SpriteRegistry, name_hash};

use crate::sprite::Quad;

/// The test sprites; `anim` has three frames.
pub const NAMES: [&str; 8] = ["tile", "unit", "rock", "mark", "step", "icon", "anim", "drop"];

/// The animated sprite's frame count.
const ANIM_FRAMES: usize = 3;

/// The registry with every one of [`NAMES`].
pub fn registry() -> SpriteRegistry {
	let sprite = |i: usize| {
		let frames = if NAMES[i] == "anim" { ANIM_FRAMES } else { 1 };
		BrushSprite {
			hash: name_hash(NAMES[i].as_bytes()),
			name: NAMES[i].into(),
			material: 0,
			width: 2,
			height: 2,
			offset_x: 0,
			offset_y: 0,
			depth_bias: 0,
			frame_time_ms: 100,
			unknown_pause: [0, 0],
			rotation_speed: 0.0,
			flags: 0,
			unknown_offset: [0.0, 0.0],
			color: [255; 4],
			unknown_scale: 1.0,
			frames: (0..frames)
				.map(|k| {
					let u = i as f32 * 0.1 + k as f32 * 0.01;
					BrushFrame {
						name: k.to_string(),
						uv: [u, u + 0.005, 0.0, 1.0],
						rect: [-1.0, 1.0, -1.0, 1.0],
					}
				})
				.collect(),
			trailer: [0; 3],
		}
	};
	let mut registry = SpriteRegistry::new();
	registry.add_sheet(
		"brushes/test.brush",
		BrushSheet {
			texture: "gfx/test.png".into(),
			header: SheetHeader {
				width: 2,
				height: 2,
				flags: [0; 12],
				sprite_count: NAMES.len() as u32,
				reserved: [0; 2],
				max_level: 1000,
			},
			sprites: (0..NAMES.len()).map(sprite).collect(),
		},
	);
	registry
}

/// The smallest `u` of a quad's corners.
fn u_of(quad: &Quad) -> f32 {
	quad.uv.iter().map(|uv| uv[0]).fold(f32::MAX, f32::min)
}

/// Which test sprite a quad shows (frames add at most 0.2 to its column, so rounding
/// finds it).
pub fn name_of(quad: &Quad) -> &'static str {
	NAMES[(u_of(quad) * 10.0).round() as usize]
}

/// Which frame of its sprite a quad shows.
pub fn frame_of(quad: &Quad) -> u32 {
	let column = u_of(quad) * 10.0;
	((column - column.round()) * 10.0).round() as u32
}
