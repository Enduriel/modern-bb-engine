//! Reading a `.brush` file into a [`BrushSheet`].

use super::{BRUSH_MAGIC, BRUSH_VERSION, BrushFrame, BrushSheet, BrushSprite, SheetHeader};
use crate::reader::Reader;

impl BrushSheet {
	/// Parse a `.brush` file. Bytes after the last sprite are ignored, as in the
	/// original. The two errors the original reports by name keep its wording.
	pub fn parse(bytes: &[u8]) -> Result<BrushSheet, String> {
		let mut r = Reader::new(bytes);
		if r.u32("magic")? != BRUSH_MAGIC {
			return Err("not a brush sheet definition".into());
		}
		let version = r.u16("version")?;
		if version != BRUSH_VERSION {
			return Err(format!(
				"brush sheet layout version incompatible: {version}"
			));
		}
		let texture = r.string("texture path")?;
		let header = read_header(&mut r)?;

		// Only the count's low 16 bits drive the original's loop.
		let count = header.sprite_count as u16;
		let mut sprites = Vec::with_capacity(usize::from(count));
		for index in 0..count {
			let sprite = read_sprite(&mut r).map_err(|e| format!("sprite {index}: {e}"))?;
			sprites.push(sprite);
		}
		Ok(BrushSheet {
			texture,
			header,
			sprites,
		})
	}
}

fn read_header(r: &mut Reader) -> Result<SheetHeader, String> {
	Ok(SheetHeader {
		width: r.u16("atlas width")?,
		height: r.u16("atlas height")?,
		flags: r.array("sheet flags")?,
		sprite_count: r.u32("sprite count")?,
		reserved: r.array("sheet reserved bytes")?,
		max_level: r.i32("sheet max level")?,
	})
}

fn read_sprite(r: &mut Reader) -> Result<BrushSprite, String> {
	let hash = r.u64("sprite hash")?;
	let name = r.string("sprite name")?;
	let material = r.u64("material")?;
	let width = r.i32("width")?;
	let height = r.i32("height")?;
	let offset_x = r.i16("offset x")?;
	let offset_y = r.i16("offset y")?;
	let depth_bias = r.i8("level")?;
	let frame_time_ms = r.u32("frame time")?;
	let unknown_pause = [r.u16("pause")?, r.u16("pause")?];
	let rotation_speed = r.f32("rotation speed")?;
	let flags = r.u32("sprite flags")?;
	let pivot = [r.f32("pivot")?, r.f32("pivot")?];
	let color = r.array("colour")?;
	let unknown_scale = r.f32("scale")?;
	let frame_count = r.u16("frame count")?;
	let mut frames = Vec::with_capacity(usize::from(frame_count));
	for _ in 0..frame_count {
		frames.push(BrushFrame {
			name: r.string("frame name")?,
			uv: [r.f32("uv")?, r.f32("uv")?, r.f32("uv")?, r.f32("uv")?],
			rect: [
				r.f32("rect")?,
				r.f32("rect")?,
				r.f32("rect")?,
				r.f32("rect")?,
			],
		});
	}
	let trailer = r.array("sprite trailer")?;
	Ok(BrushSprite {
		hash,
		name,
		material,
		width,
		height,
		offset_x,
		offset_y,
		depth_bias,
		frame_time_ms,
		unknown_pause,
		rotation_speed,
		flags,
		pivot,
		color,
		unknown_scale,
		frames,
		trailer,
	})
}

#[cfg(test)]
mod tests {
	use super::super::test_data::{SheetBuilder, sprite};
	use super::*;
	use crate::name_hash;

	#[test]
	fn parses_every_field_in_order() {
		let mut original = sprite("bust_head_01");
		original.offset_x = -3;
		original.offset_y = 7;
		original.depth_bias = -16;
		original.frame_time_ms = 200;
		original.unknown_pause = [800, 9000];
		original.rotation_speed = 12.5;
		original.flags = 0x1403;
		original.pivot = [-15.0, -30.0];
		original.color = [41, 28, 20, 255];
		original.frames.push(BrushFrame {
			name: "second.png".into(),
			uv: [0.5, 0.75, 0.25, 0.5],
			rect: [-10.0, 9.0, -20.0, 19.0],
		});
		original.trailer = [1, 2, 3];
		let bytes = SheetBuilder::new("gfx/entity_0.png")
			.sprite(original.clone())
			.build();

		let sheet = BrushSheet::parse(&bytes).unwrap();
		assert_eq!(sheet.texture, "gfx/entity_0.png");
		assert_eq!(sheet.header.width, 2048);
		assert_eq!(sheet.header.sprite_count, 1);
		assert_eq!(sheet.header.max_level, 1000);
		assert_eq!(sheet.sprites, vec![original]);
	}

	#[test]
	fn stores_the_hash_as_written_without_rehashing() {
		let mut odd = sprite("renamed");
		odd.hash = 42;
		let bytes = SheetBuilder::new("gfx/a.png").sprite(odd).build();
		let sheet = BrushSheet::parse(&bytes).unwrap();
		assert_eq!(sheet.sprites[0].hash, 42);
		assert_ne!(name_hash(b"renamed"), 42);
	}

	#[test]
	fn rejects_a_wrong_magic_number() {
		let mut bytes = SheetBuilder::new("gfx/a.png").build();
		bytes[0] ^= 1;
		assert_eq!(
			BrushSheet::parse(&bytes),
			Err("not a brush sheet definition".to_string())
		);
	}

	#[test]
	fn rejects_other_layout_versions() {
		let mut bytes = SheetBuilder::new("gfx/a.png").build();
		bytes[4] = 16;
		assert_eq!(
			BrushSheet::parse(&bytes),
			Err("brush sheet layout version incompatible: 16".to_string())
		);
	}

	#[test]
	fn a_truncated_sprite_is_an_error_naming_it() {
		let mut bytes = SheetBuilder::new("gfx/a.png")
			.sprite(sprite("a"))
			.sprite(sprite("b"))
			.build();
		bytes.truncate(bytes.len() - 5);
		let err = BrushSheet::parse(&bytes).unwrap_err();
		assert!(err.starts_with("sprite 1: truncated"), "{err}");
	}

	#[test]
	fn reads_only_the_low_16_bits_of_the_sprite_count() {
		let bytes = SheetBuilder::new("gfx/a.png")
			.sprite(sprite("a"))
			.count(0x1_0001)
			.build();
		let sheet = BrushSheet::parse(&bytes).unwrap();
		assert_eq!(sheet.header.sprite_count, 0x1_0001);
		assert_eq!(sheet.sprites.len(), 1);
	}

	#[test]
	fn ignores_bytes_after_the_last_sprite() {
		let mut bytes = SheetBuilder::new("gfx/a.png").sprite(sprite("a")).build();
		bytes.extend_from_slice(b"junk");
		assert_eq!(BrushSheet::parse(&bytes).unwrap().sprites.len(), 1);
	}

	#[test]
	fn bounds_start_at_the_anchor_and_grow_with_each_frame() {
		let mut s = sprite("a");
		s.frames[0].rect = [2.0, 5.0, 1.0, 4.0];
		assert_eq!(s.bounds(), [0.0, 5.0, 0.0, 4.0]);
		s.frames.push(BrushFrame {
			name: String::new(),
			uv: [0.0; 4],
			rect: [-6.0, 1.0, -7.0, 9.0],
		});
		assert_eq!(s.bounds(), [-6.0, 5.0, -7.0, 9.0]);
	}
}
