//! The game's cursor sprites as pictures for the system to show (a hardware cursor),
//! one per animation frame, looking as the software cursor draws them
//! ([`crate::cursor`]): the frame's texels, unscaled and unlit, with the hotspot where
//! the mouse is.
//!
//! The software cursor puts the brush's anchor at the mouse plus the brush's offset
//! (y up), and the frame's rect around the anchor. So the picture's top-left pixel is
//! `offset_x + rect.left` right of the mouse's pixel and `offset_y + rect.top(y up)`
//! above it, and the hotspot is that, negated. A system cursor's hotspot must lie in
//! its picture: one that falls outside (a brush whose picture is away from the mouse)
//! is reached with transparent pixels added on that side.

use bb_assets::{BrushFrame, BrushSprite, CursorImage, Image};

/// The largest cursor picture, in pixels a side, shown by the system: the platforms'
/// cursors go up to 256 (Windows' largest icon; X11 and Wayland compositors take
/// bigger ones, but not everywhere on a hardware plane). Vanilla's are 32 and 42.
pub const MAX_SIZE: u32 = 256;

/// The pictures of `sprite`'s frames, in order, cut from its sheet's `atlas` (as decoded, the
/// first row the bottom of the pictures), or `None` if a frame has no pixels or is
/// larger than [`MAX_SIZE`] once its hotspot is in it.
pub fn cursor_frames(sprite: &BrushSprite, atlas: &Image) -> Option<Vec<CursorImage>> {
	if sprite.frames.is_empty() {
		return None;
	}
	sprite.frames.iter().map(|frame| frame_image(sprite, frame, atlas)).collect()
}

/// One frame's picture: each pixel the texel the software cursor samples at its centre
/// (drawn whole pixels from the mouse, a pixel's centre is a texel's).
fn frame_image(sprite: &BrushSprite, frame: &BrushFrame, atlas: &Image) -> Option<CursorImage> {
	let [left, right, bottom, top] = frame.rect;
	let [u0, u1, v0, v1] = frame.uv;
	let (width, height) = ((right - left).round(), (top - bottom).round());
	if width < 1.0 || height < 1.0 || atlas.width == 0 || atlas.height == 0 {
		return None;
	}
	let (width, height) = (width as u32, height as u32);
	// Where the picture's top-left pixel is from the mouse's, y down.
	let dx = (f32::from(sprite.offset_x) + left).round() as i64;
	let dy = -(f32::from(sprite.offset_y) + top).round() as i64;
	// The picture with the mouse's pixel in it: transparent pixels added where needed.
	let (pad_left, pad_top) = ((dx.max(0)) as u32, (dy.max(0)) as u32);
	let full_width = (pad_left + width).max((-dx + 1).max(0) as u32);
	let full_height = (pad_top + height).max((-dy + 1).max(0) as u32);
	if full_width > MAX_SIZE || full_height > MAX_SIZE {
		return None;
	}
	let mut rgba = vec![0u8; (full_width * full_height * 4) as usize];
	let texel = |u: f32, v: f32| -> [u8; 4] {
		let x = ((u * atlas.width as f32).floor() as i64).clamp(0, i64::from(atlas.width) - 1) as usize;
		let y = ((v * atlas.height as f32).floor() as i64).clamp(0, i64::from(atlas.height) - 1) as usize;
		let at = (y * atlas.width as usize + x) * 4;
		atlas.rgba[at..at + 4].try_into().expect("four bytes")
	};
	for row in 0..height {
		// Pictures are stored upside down: the top row is towards `v1`.
		let v = v1 + (v0 - v1) * ((row as f32 + 0.5) / height as f32);
		for column in 0..width {
			let u = u0 + (u1 - u0) * ((column as f32 + 0.5) / width as f32);
			let at = (((pad_top + row) * full_width + pad_left + column) * 4) as usize;
			rgba[at..at + 4].copy_from_slice(&texel(u, v));
		}
	}
	Some(CursorImage {
		width: full_width,
		height: full_height,
		hotspot: ((pad_left as i64 - dx) as u16, (pad_top as i64 - dy) as u16),
		rgba,
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A 4 × 4 atlas whose texel `(x, y)` (row `y` of the file) is `[x, y, 7, 255]`.
	fn atlas() -> Image {
		let rgba = (0..4u8).flat_map(|y| (0..4u8).flat_map(move |x| [x, y, 7, 255])).collect();
		Image { width: 4, height: 4, rgba }
	}

	fn sprite(offset: (i16, i16), rect: [f32; 4], uv: [f32; 4], frames: usize) -> BrushSprite {
		BrushSprite {
			hash: 0,
			name: "cursor_test".into(),
			material: 0,
			width: 4,
			height: 4,
			offset_x: offset.0,
			offset_y: offset.1,
			depth_bias: 0,
			frame_time_ms: 50,
			unknown_pause: [0, 0],
			rotation_speed: 0.0,
			flags: 0,
			unknown_offset: [0.0, 0.0],
			color: [255; 4],
			unknown_scale: 1.0,
			frames: (0..frames)
				.map(|i| BrushFrame {
					name: format!("{i}"),
					uv,
					rect,
				})
				.collect(),
			trailer: [0; 3],
		}
	}

	fn pixel(image: &CursorImage, x: u32, y: u32) -> [u8; 4] {
		let at = ((y * image.width + x) * 4) as usize;
		image.rgba[at..at + 4].try_into().unwrap()
	}

	#[test]
	fn a_centred_cursor_has_its_hotspot_in_the_middle_and_shows_upright() {
		// The whole atlas, centred on the anchor, which is the mouse.
		let s = sprite((0, 0), [-2.0, 2.0, -2.0, 2.0], [0.0, 1.0, 0.0, 1.0], 1);
		let frames = cursor_frames(&s, &atlas()).expect("a picture");
		let image = &frames[0];
		assert_eq!((image.width, image.height, image.hotspot), (4, 4, (2, 2)));
		// The atlas's last row is the picture's top.
		assert_eq!(pixel(image, 0, 0), [0, 3, 7, 255]);
		assert_eq!(pixel(image, 3, 3), [3, 0, 7, 255]);
	}

	#[test]
	fn the_brush_offset_moves_the_hotspot_as_vanillas_hand_does() {
		// `cursor_hand`: offset (16, -14), rect [-16, 16, -10, 16]: the tip two pixels
		// below the picture's top-left corner.
		let s = sprite((16, -14), [-16.0, 16.0, -10.0, 16.0], [0.0, 1.0, 0.0, 1.0], 1);
		let image = &cursor_frames(&s, &atlas()).unwrap()[0];
		assert_eq!((image.width, image.height, image.hotspot), (32, 26, (0, 2)));
		// Nearest texel: the top rows come from the atlas's last.
		assert_eq!(pixel(image, 0, 0), [0, 3, 7, 255]);
		assert_eq!(pixel(image, 31, 25), [3, 0, 7, 255]);
	}

	#[test]
	fn a_picture_away_from_the_mouse_gets_room_for_its_hotspot() {
		// Right of and below the mouse: one column and one row added before it.
		let s = sprite((3, -3), [-2.0, 2.0, -2.0, 2.0], [0.0, 1.0, 0.0, 1.0], 1);
		let image = &cursor_frames(&s, &atlas()).unwrap()[0];
		assert_eq!((image.width, image.height, image.hotspot), (5, 5, (0, 0)));
		assert_eq!(pixel(image, 0, 0), [0; 4], "transparent");
		assert_eq!(pixel(image, 1, 1), [0, 3, 7, 255]);
		// Left of and above it, ending before it: room after it.
		let s = sprite((-4, 4), [-2.0, 2.0, -2.0, 2.0], [0.0, 1.0, 0.0, 1.0], 1);
		let image = &cursor_frames(&s, &atlas()).unwrap()[0];
		assert_eq!((image.width, image.height, image.hotspot), (7, 7, (6, 6)));
		assert_eq!(pixel(image, 0, 0), [0, 3, 7, 255]);
	}

	#[test]
	fn every_frame_is_a_picture_and_too_large_is_none() {
		let s = sprite((0, 0), [-1.0, 1.0, -1.0, 1.0], [0.0, 0.5, 0.0, 0.5], 10);
		let frames = cursor_frames(&s, &atlas()).unwrap();
		assert_eq!(frames.len(), 10);
		assert_eq!(pixel(&frames[9], 0, 0), [0, 1, 7, 255], "a quarter of the atlas");
		let big = sprite((0, 0), [-200.0, 200.0, -1.0, 1.0], [0.0, 1.0, 0.0, 1.0], 1);
		assert_eq!(cursor_frames(&big, &atlas()), None);
		let far = sprite((300, 0), [-1.0, 1.0, -1.0, 1.0], [0.0, 1.0, 0.0, 1.0], 1);
		assert_eq!(cursor_frames(&far, &atlas()), None, "the hotspot too far from it");
	}
}
