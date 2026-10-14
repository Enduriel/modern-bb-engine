//! Building `.brush` bytes for tests, in the layout the parser reads.

use super::{BRUSH_MAGIC, BRUSH_VERSION, BrushFrame, BrushSprite};
use crate::name_hash;

/// A still sprite with the values most vanilla sprites store.
pub(crate) fn sprite(name: &str) -> BrushSprite {
	BrushSprite {
		hash: name_hash(name.as_bytes()),
		name: name.into(),
		material: 0xbf79_5ef4_d3fe_85c5,
		width: 20,
		height: 10,
		offset_x: 0,
		offset_y: 0,
		depth_bias: 0,
		frame_time_ms: 100,
		unknown_pause: [0, 0],
		rotation_speed: 0.0,
		flags: 0x403,
		unknown_offset: [0.0, 0.0],
		color: [255, 255, 255, 255],
		unknown_scale: 2.0,
		frames: vec![BrushFrame {
			name: format!("{name}.png"),
			uv: [0.0, 0.25, 0.0, 0.125],
			rect: [-10.0, 10.0, -5.0, 5.0],
		}],
		trailer: [0, 0, 0],
	}
}

/// Writes a sheet's bytes. The sprite count defaults to the number of sprites.
pub(crate) struct SheetBuilder {
	texture: String,
	sprites: Vec<BrushSprite>,
	count: Option<u32>,
}

impl SheetBuilder {
	pub(crate) fn new(texture: &str) -> Self {
		SheetBuilder {
			texture: texture.into(),
			sprites: Vec::new(),
			count: None,
		}
	}

	pub(crate) fn sprite(mut self, sprite: BrushSprite) -> Self {
		self.sprites.push(sprite);
		self
	}

	/// Store `count` as the sprite count instead of the real number.
	pub(crate) fn count(mut self, count: u32) -> Self {
		self.count = Some(count);
		self
	}

	pub(crate) fn build(&self) -> Vec<u8> {
		let mut out = Vec::new();
		out.extend_from_slice(&BRUSH_MAGIC.to_le_bytes());
		out.extend_from_slice(&BRUSH_VERSION.to_le_bytes());
		string(&mut out, &self.texture);
		out.extend_from_slice(&2048u16.to_le_bytes());
		out.extend_from_slice(&1024u16.to_le_bytes());
		out.extend_from_slice(&[1, 0, 1, 1, 1, 0, 0, 0, 1, 0, 0, 0]);
		let count = self.count.unwrap_or(self.sprites.len() as u32);
		out.extend_from_slice(&count.to_le_bytes());
		out.extend_from_slice(&[0, 0]);
		out.extend_from_slice(&1000i32.to_le_bytes());
		for s in &self.sprites {
			out.extend_from_slice(&s.hash.to_le_bytes());
			string(&mut out, &s.name);
			out.extend_from_slice(&s.material.to_le_bytes());
			out.extend_from_slice(&s.width.to_le_bytes());
			out.extend_from_slice(&s.height.to_le_bytes());
			out.extend_from_slice(&s.offset_x.to_le_bytes());
			out.extend_from_slice(&s.offset_y.to_le_bytes());
			out.push(s.depth_bias as u8);
			out.extend_from_slice(&s.frame_time_ms.to_le_bytes());
			for value in s.unknown_pause {
				out.extend_from_slice(&value.to_le_bytes());
			}
			out.extend_from_slice(&s.rotation_speed.to_le_bytes());
			out.extend_from_slice(&s.flags.to_le_bytes());
			for value in s.unknown_offset {
				out.extend_from_slice(&value.to_le_bytes());
			}
			out.extend_from_slice(&s.color);
			out.extend_from_slice(&s.unknown_scale.to_le_bytes());
			out.extend_from_slice(&(s.frames.len() as u16).to_le_bytes());
			for frame in &s.frames {
				string(&mut out, &frame.name);
				for value in frame.uv.iter().chain(&frame.rect) {
					out.extend_from_slice(&value.to_le_bytes());
				}
			}
			out.extend_from_slice(&s.trailer);
		}
		out
	}
}

fn string(out: &mut Vec<u8>, s: &str) {
	out.extend_from_slice(&(s.len() as u16).to_le_bytes());
	out.extend_from_slice(s.as_bytes());
}
