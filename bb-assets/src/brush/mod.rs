//! `.brush` files: the game's sprite sheets.
//!
//! A brush file describes one atlas image (`gfx/*.png`) and the named sprites cut
//! from it. Scripts refer to sprites only by name (`setBrush("bust_head_01")`,
//! `doesBrushExist(..)`, `spawnDetail(..)`), and the engine resolves the name through
//! a registry of every sprite in every loaded sheet ([`crate::SpriteRegistry`]).
//!
//! The layout below follows the original's reader, disassembled from the
//! Steamless-unpacked exe: the sheet loader is at `0x572220`, the sprite record
//! reader at `0x579f80` and the per-sheet texture setup at `0x579ca0`. The field
//! *order and widths* are exact (every vanilla and installed-mod brush parses to its
//! last byte). Adam Milazzo's `bbrusher` reads the same bytes, but it splits some
//! fields differently. Where a field's meaning isn't known yet, its name says so and
//! its doc gives what the data shows. The renderer will settle those.
//!
//! ```text
//! u32 magic 0xBAADFAAD, u16 version 17, string texture
//! u16 width, u16 height, [u8; 12] flags, u32 sprite count, [u8; 2], i32
//! per sprite:
//!   u64 hash, string name, u64 material, i32 width, i32 height, i16 x, i16 y,
//!   i8, u32 frame time, [u16; 2], f32 rotation speed, u32 flags, [f32; 2],
//!   [u8; 4] colour, f32, u16 frame count,
//!   per frame: string name, [f32; 4] uv, [f32; 4] rect
//!   u8, u16 (read and dropped)
//! ```
//!
//! Strings are a `u16` byte length followed by the bytes. All values are
//! little-endian.

mod parse;
#[cfg(test)]
pub(crate) mod test_data;

/// The magic number every `.brush` file starts with.
pub const BRUSH_MAGIC: u32 = 0xBAAD_FAAD;

/// The only layout version the original accepts.
pub const BRUSH_VERSION: u16 = 17;

/// One `.brush` file: an atlas image and the sprites cut from it.
#[derive(Clone, Debug, PartialEq)]
pub struct BrushSheet {
	/// Path of the atlas image in the game's file system, e.g. `gfx/entity_0.png`.
	pub texture: String,
	/// The atlas size and texture settings.
	pub header: SheetHeader,
	/// The sprites, in file order.
	pub sprites: Vec<BrushSprite>,
}

/// The 26 bytes after the texture path: the atlas size and how the texture is set up.
#[derive(Clone, Debug, PartialEq)]
pub struct SheetHeader {
	/// Atlas width in pixels, as stored.
	pub width: u16,
	/// Atlas height in pixels, as stored.
	pub height: u16,
	/// Byte flags. When the original binds the atlas it copies seven of them into the
	/// texture's flag word: byte 1 → `0x1`, 2 → `0x400`, 3 → `0x800`, 4 → `0x2`,
	/// 6 → `0x1000`, 7 → `0x4`, 9 → `0x2000`. Byte 5 set clears sprite flag `0x2`.
	/// Bytes 0, 8, 10 and 11 aren't read there; byte 0 is 1 on most sheets and 2, 6,
	/// 8 or 40 on the UI, terrain, tile and decal sheets.
	pub flags: [u8; 12],
	/// The sprite count as stored. The original reads only its low 16 bits.
	pub sprite_count: u32,
	/// Two bytes the original doesn't read here (0 in every shipped sheet).
	pub reserved: [u8; 2],
	/// Stored on the texture (its low 16 bits). 1000 on most sheets, 4 or 5 on the
	/// terrain, transition, world-tile and world-decal sheets. 1000 is OpenGL's default
	/// `GL_TEXTURE_MAX_LEVEL`, so this is probably the highest mip level used.
	pub max_level: i32,
}

/// One named sprite: its size, anchor offset and animation frames.
#[derive(Clone, Debug, PartialEq)]
pub struct BrushSprite {
	/// The stored [`name_hash`](crate::name_hash) of `name`. The original keys its
	/// registry on this stored value; it never re-hashes the name.
	pub hash: u64,
	/// The name scripts use, e.g. `bust_head_01`.
	pub name: String,
	/// A hash the original resolves through a second registry when the sheet loads.
	/// Every vanilla and installed-mod sprite has `0xbf79_5ef4_d3fe_85c5`.
	pub material: u64,
	/// Width in pixels.
	pub width: i32,
	/// Height in pixels.
	pub height: i32,
	/// Horizontal draw offset in pixels (the original converts it to `f32`).
	pub offset_x: i16,
	/// Vertical draw offset in pixels.
	pub offset_y: i16,
	/// Unknown. 0 on most sprites, -16 to -1 on corpse parts (`*_dead`), 3 to 32 on
	/// ground details.
	pub unknown_level: i8,
	/// Time per animation frame in ms: 100 by default, 50 on the 10-frame attack
	/// cursor, 200 on the 3-frame birds.
	pub frame_time_ms: u32,
	/// Unknown. 0 except on the swamp bubbles (800 and 9000), which suggests the
	/// bounds of a random pause between animation loops, in ms.
	pub unknown_pause: [u16; 2],
	/// Rotation speed (bbrusher's `rotSpeed`). Set on the fog rotator, flies and
	/// leaves; 0 otherwise.
	pub rotation_speed: f32,
	/// Flag bits. The original's default before reading is `0x402`; most sprites
	/// store `0x403`.
	pub flags: u32,
	/// Unknown. `(0, 0)` on most sprites, mostly `(-15, -15)` on corpse parts and
	/// blood pools.
	pub unknown_offset: [f32; 2],
	/// An RGBA colour. The original also stores a grey level derived from it.
	pub color: [u8; 4],
	/// Unknown. 2.0 in every shipped sprite.
	pub unknown_scale: f32,
	/// The animation frames (one for a still sprite).
	pub frames: Vec<BrushFrame>,
	/// Three bytes after the frames that the original reads and drops.
	pub trailer: [u8; 3],
}

/// One animation frame: where it sits in the atlas and the quad it covers.
#[derive(Clone, Debug, PartialEq)]
pub struct BrushFrame {
	/// The source image the packer used, e.g. `_debug\debug_circle_100.png`. The
	/// original keeps it only in a debug mode.
	pub name: String,
	/// Atlas coordinates as stored: left, right, top, bottom, each in `0..=1`.
	/// bbrusher packs atlases upside down, so "top" is the lower row.
	pub uv: [f32; 4],
	/// The quad's edges in pixels around the sprite's anchor: left, right, top,
	/// bottom. Packers centre it: `-(w + 1) / 2`, `w / 2`, `-(h + 1) / 2`, `h / 2`.
	pub rect: [f32; 4],
}

impl BrushSprite {
	/// The union of the frames' rectangles and the anchor (left, right, top,
	/// bottom), as the original computes it while reading the frames: starting from
	/// zero, each frame can only widen the box.
	pub fn bounds(&self) -> [f32; 4] {
		let mut bounds = [0.0f32; 4];
		for frame in &self.frames {
			let [left, right, top, bottom] = frame.rect;
			if bounds[0] > left {
				bounds[0] = left;
			}
			if right > bounds[1] {
				bounds[1] = right;
			}
			if bounds[2] > top {
				bounds[2] = top;
			}
			if bottom > bounds[3] {
				bounds[3] = bottom;
			}
		}
		bounds
	}
}
