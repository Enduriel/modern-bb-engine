//! The units' orientation overlays (see `squirrel_bb::native::camera_overlays`): the
//! battle camera's `render()` draws each shown one as a screen-space sprite
//! (`0x4a4030` → `0x52c490`), after the battle and before the UI, at its place in
//! pixels from the screen's centre plus its brush's offset, scaled by its own scale
//! (a banner's 3/4) and not by the camera's zoom, unlit, with its colour's alpha.

use bb_assets::SpriteRegistry;
use squirrel_bb::scene::tactical::SceneOrientationOverlay;

use crate::sprite::Quad;
use crate::world::{AtlasList, Look, WorldQuads, frame_quad, unlit};

/// The quads of `overlays` on a target `fit` times the game's screen (its pixels per
/// screen pixel); `None` when none can be drawn.
pub fn orientation_quads(registry: &SpriteRegistry, overlays: &[SceneOrientationOverlay], fit: f32) -> Option<WorldQuads> {
	let mut atlases = AtlasList::default();
	let mut quads: Vec<Quad> = Vec::new();
	for o in overlays {
		let Some((sheet, sprite)) = registry.get(&o.brush) else { continue };
		let Some(frame) = sprite.frames.first() else { continue };
		let scale = o.scale * fit;
		let anchor = [
			o.pos.0 * fit + f32::from(sprite.offset_x) * scale,
			o.pos.1 * fit + f32::from(sprite.offset_y) * scale,
		];
		let [a, r, g, b] = o.color.to_be_bytes();
		let look = Look {
			scale,
			alpha: f32::from(a),
			tint: [r, g, b],
			..Look::PLAIN
		};
		quads.push(Quad {
			texture: atlases.index_of(&sheet.sheet.texture),
			flags: unlit(true),
			..frame_quad(frame, anchor, look)
		});
	}
	if quads.is_empty() {
		return None;
	}
	let labels_from = quads.len();
	Some(WorldQuads {
		quads,
		atlases,
		labels_from,
		details_end: 0,
		details_keys: Vec::new(),
	})
}

#[cfg(test)]
mod tests {
	use bb_assets::{BrushFrame, BrushSheet, BrushSprite, SheetHeader, name_hash};

	use super::*;

	fn registry() -> SpriteRegistry {
		let sprite = |name: &str, offset_y: i16| BrushSprite {
			hash: name_hash(name.as_bytes()),
			name: name.into(),
			material: 0,
			width: 46,
			height: 46,
			offset_x: 0,
			offset_y,
			depth_bias: 0,
			frame_time_ms: 0,
			unknown_pause: [0, 0],
			rotation_speed: 0.0,
			flags: 0,
			pivot: [0.0, 0.0],
			color: [255; 4],
			unknown_scale: 1.0,
			frames: vec![BrushFrame {
				name: "0".into(),
				uv: [0.0, 1.0, 0.0, 1.0],
				rect: [-23.0, 23.0, -23.0, 23.0],
			}],
			trailer: [0; 3],
		};
		let mut registry = SpriteRegistry::new();
		registry.add_sheet(
			"brushes/orientation_overlay.brush",
			BrushSheet {
				texture: "gfx/orientation_overlay.png".into(),
				header: SheetHeader {
					width: 64,
					height: 64,
					flags: [0; 12],
					sprite_count: 2,
					reserved: [0; 2],
					max_level: 1000,
				},
				sprites: vec![sprite("zombie_01_orientation", 0), sprite("banner_06", 27)],
			},
		);
		registry
	}

	fn overlay(brush: &str, scale: f32) -> SceneOrientationOverlay {
		SceneOrientationOverlay {
			brush: brush.into(),
			pos: (-300.0, -30.0),
			scale,
			color: 0x80ff_ffff,
		}
	}

	#[test]
	fn a_portrait_is_drawn_unlit_at_its_place_with_its_alpha() {
		let map = orientation_quads(&registry(), &[overlay("zombie_01_orientation", 1.0)], 1.0).unwrap();
		let q = &map.quads[0];
		assert_eq!(q.corners[0], [-323.0, -7.0]);
		assert_eq!(q.corners[2], [-277.0, -53.0]);
		assert_eq!(q.color, [255, 255, 255, 0x80]);
		assert_eq!(q.flags, 1);
	}

	#[test]
	fn a_banner_is_drawn_at_its_scale_with_its_offset_scaled() {
		let map = orientation_quads(&registry(), &[overlay("banner_06", 0.75)], 2.0).unwrap();
		let q = &map.quads[0];
		// Centre (-600, -60) + 27 · 1.5 up = (-600, -19.5); half size 23 · 1.5.
		assert_eq!(q.corners[0], [-634.5, 15.0]);
		assert_eq!(q.corners[2], [-565.5, -54.0]);
	}

	#[test]
	fn an_unknown_brush_draws_nothing() {
		assert!(orientation_quads(&registry(), &[overlay("nope", 1.0)], 1.0).is_none());
	}
}
