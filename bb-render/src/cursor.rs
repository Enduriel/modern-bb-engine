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
	let frames = sprite.frames.len();
	if frames <= 1 || sprite.frame_time_ms == 0 {
		return sprite.frames.first();
	}
	let ms = (since.max(0.0) * 1000.0) as u64;
	sprite.frames.get((ms / u64::from(sprite.frame_time_ms)) as usize % frames)
}

/// The cursor `brush`'s quad at `at` (from the screen's centre, y up), `since` seconds
/// into its animation; `None` for an unknown brush.
pub fn cursor_quads(registry: &SpriteRegistry, brush: &str, since: f32, at: [f32; 2]) -> Option<WorldQuads> {
	let (sheet, sprite) = registry.get(brush)?;
	let frame = frame_at(sprite, since)?;
	let anchor = [at[0] + f32::from(sprite.offset_x), at[1] + f32::from(sprite.offset_y)];
	let mut atlases = AtlasList::default();
	let quad = Quad {
		texture: atlases.index_of(&sheet.sheet.texture),
		flags: 1,
		..frame_quad(frame, anchor, Look::PLAIN)
	};
	Some(WorldQuads {
		quads: vec![quad],
		atlases,
		labels_from: 1,
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
	}
}
