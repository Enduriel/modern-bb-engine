//! The mouse cursor's sprite, as the original's software cursor draws it (`UICursor`
//! `0x52C430`): one frame of its `ui.brush` sprite, at the
//! position the scripts set plus the brush's offset, in screen pixels from the centre
//! with y up, unscaled and unlit. An animated cursor (the attack cursor's ten frames of
//! 50 ms) loops from when it was set, on virtual time.

use bb_assets::{BrushFrame, BrushSprite, SpriteRegistry};

use crate::sprite::Quad;
use crate::world::{AtlasList, Look, WorldQuads, frame_quad};

/// The frame of `sprite` that shows `since` seconds into its animation.
pub fn frame_at(sprite: &BrushSprite, since: f32) -> Option<&BrushFrame> {
	sprite.frames.get(frame_index_at(sprite, since))
}

/// Which of `sprite`'s frames shows `since` seconds into its animation (0 for a
/// still one).
pub fn frame_index_at(sprite: &BrushSprite, since: f32) -> usize {
	let frames = sprite.frames.len();
	if frames <= 1 || sprite.frame_time_ms == 0 {
		return 0;
	}
	let ms = (since.max(0.0) * 1000.0) as u64;
	(ms / u64::from(sprite.frame_time_ms)) as usize % frames
}

/// The cursor `brush`'s quad at `at` (from the screen's centre, y up), `since` seconds
/// into its animation, `scale` frame pixels to a pixel of its picture (1: unscaled);
/// `None` for an unknown brush.
pub fn cursor_quads(registry: &SpriteRegistry, brush: &str, since: f32, at: [f32; 2], scale: f32) -> Option<WorldQuads> {
	let (sheet, sprite) = registry.get(brush)?;
	let frame = frame_at(sprite, since)?;
	let anchor = [at[0] + f32::from(sprite.offset_x) * scale, at[1] + f32::from(sprite.offset_y) * scale];
	let mut atlases = AtlasList::default();
	let quad = Quad {
		texture: atlases.index_of(&sheet.sheet.texture),
		flags: 1,
		..frame_quad(frame, anchor, Look { scale, ..Look::PLAIN })
	};
	Some(WorldQuads {
		quads: vec![quad],
		atlases,
		labels_from: 1,
		details_end: 0,
		details_keys: Vec::new(),
	})
}

#[cfg(test)]
mod tests {
	use bb_assets::BrushFrame;

	use super::*;

	fn sprite(frames: usize, frame_time_ms: u32) -> BrushSprite {
		BrushSprite {
			hash: 0,
			name: "cursor_attack".into(),
			material: 0,
			width: 32,
			height: 32,
			offset_x: 21,
			offset_y: -19,
			depth_bias: 0,
			frame_time_ms,
			unknown_pause: [0, 0],
			rotation_speed: 0.0,
			flags: 0x13,
			unknown_offset: [0.0, 0.0],
			color: [255; 4],
			unknown_scale: 1.0,
			frames: (0..frames)
				.map(|i| BrushFrame {
					name: format!("{i}"),
					uv: [0.0, 1.0, 0.0, 1.0],
					rect: [-16.0, 16.0, -16.0, 16.0],
				})
				.collect(),
			trailer: [0; 3],
		}
	}

	#[test]
	fn an_animated_cursor_loops_its_frames() {
		let s = sprite(10, 50);
		assert_eq!(frame_at(&s, 0.0).unwrap().name, "0");
		assert_eq!(frame_at(&s, 0.049).unwrap().name, "0");
		assert_eq!(frame_at(&s, 0.05).unwrap().name, "1");
		assert_eq!(frame_at(&s, 0.52).unwrap().name, "0", "ten frames, then again");
		assert_eq!(frame_at(&sprite(1, 0), 3.0).unwrap().name, "0");
		assert_eq!(frame_index_at(&s, 0.12), 2);
		assert_eq!(frame_index_at(&sprite(0, 50), 1.0), 0, "no frames");
	}

	/// At a Retina display's scale, the picture and its offset are twice as large, so it
	/// covers what it would on a display without Retina.
	#[test]
	fn a_scaled_cursor_covers_its_picture_times_the_scale() {
		use bb_assets::{BrushSheet, SheetHeader};
		let mut registry = SpriteRegistry::new();
		registry.add_sheet(
			"brushes/ui.brush",
			BrushSheet {
				texture: "gfx/ui.png".into(),
				header: SheetHeader {
					width: 64,
					height: 64,
					flags: [0; 12],
					sprite_count: 1,
					reserved: [0; 2],
					max_level: 1000,
				},
				sprites: vec![BrushSprite {
					hash: bb_assets::name_hash(b"cursor_attack"),
					..sprite(1, 0)
				}],
			},
		);
		let bounds = |scale: f32| {
			let map = cursor_quads(&registry, "cursor_attack", 0.0, [100.0, 50.0], scale).expect("a quad");
			let corners = map.quads[0].corners;
			let xs = corners.iter().map(|c| c[0]);
			let ys = corners.iter().map(|c| c[1]);
			(xs.clone().fold(f32::MAX, f32::min), xs.fold(f32::MIN, f32::max), ys.clone().fold(f32::MAX, f32::min), ys.fold(f32::MIN, f32::max))
		};
		// Offset (21, -19), a 32x32 picture about it.
		assert_eq!(bounds(1.0), (105.0, 137.0, 15.0, 47.0));
		assert_eq!(bounds(2.0), (110.0, 174.0, -20.0, 44.0));
	}
}
