//! Picking: which battle tile a world point falls on (`Tactical.worldToTile`), as the
//! original computes it (disassembled; matches 31,880 probed points, raised tiles
//! included).
//!
//! The point's cell comes from plain arithmetic, then the game's hex masks
//! (`gfx/masks/hex_mask.0`–`.3`, one per level: a 160 px wide tile image, 40 px
//! taller per level for the wall below a raised tile) decide the rest. Raised tiles
//! in front of the cell are hit-tested first, front to back, then the cell's own mask
//! says whether the point is on a corner that belongs to a neighbour. A tile counts
//! as raised only up to the camera's `Level`. All arithmetic is `f32`, as in the
//! original: tie points on hex edges depend on it.

use super::TacticalGridState;

/// Tile width, row height and level height in world pixels.
const TILE_W: f32 = 160.0;
const ROW_H: f32 = 80.0;
const LEVEL_H: f32 = 40.0;
/// Horizontal spacing of columns (3/4 of the width) and the half-row shift per column.
const COL_W: f32 = TILE_W * 0.75;
const HALF_ROW: f32 = ROW_H * 0.5;

/// What one mask pixel says: the tile itself, a neighbour's corner, or nothing.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pixel {
	Tile,
	/// Belongs to the neighbour at this axial offset.
	Neighbour(i64, i64),
	None,
}

/// The four hex masks, one per level, decoded from the game's TGA files.
pub(crate) struct HexMasks {
	width: usize,
	/// Per level: (height, pixels top row first).
	levels: Vec<(usize, Vec<Pixel>)>,
}

impl HexMasks {
	/// Read and decode `gfx/masks/hex_mask.0` to `.3`, or `None` if any is missing or
	/// not a mask the original could use.
	pub(crate) fn load(read: &dyn Fn(&str) -> Option<Vec<u8>>) -> Option<HexMasks> {
		let mut levels = Vec::new();
		let mut width = 0;
		for level in 0..4 {
			let (w, h, pixels) = decode_tga(&read(&format!("gfx/masks/hex_mask.{level}"))?)?;
			if w != TILE_W as usize || h != (ROW_H + LEVEL_H * level as f32) as usize {
				return None;
			}
			width = w;
			levels.push((h, pixels));
		}
		Some(HexMasks { width, levels })
	}

	fn pixel(&self, level: usize, x: i32, y: i32) -> Pixel {
		let (h, pixels) = &self.levels[level];
		if x < 0 || y < 0 || x as usize >= self.width || y as usize >= *h {
			return Pixel::None;
		}
		pixels[y as usize * self.width + x as usize]
	}
}

/// An uncompressed 32-bit TGA as (width, height, pixels top row first). The colours
/// are the ones the original compares with: black is the tile, and blue, green, red
/// and white mark the corners of the neighbours at (+1, 0), (-1, 0), (-1, +1) and
/// (+1, -1).
fn decode_tga(d: &[u8]) -> Option<(usize, usize, Vec<Pixel>)> {
	let header = d.get(..18)?;
	if header[1] != 0 || header[2] != 2 || header[16] != 32 {
		return None;
	}
	let w = u16::from_le_bytes([header[12], header[13]]) as usize;
	let h = u16::from_le_bytes([header[14], header[15]]) as usize;
	let top_first = header[17] & 0x20 != 0;
	let data = d.get(18 + header[0] as usize..)?.get(..w * h * 4)?;
	let mut pixels = Vec::with_capacity(w * h);
	for y in 0..h {
		let row = if top_first { y } else { h - 1 - y };
		for x in 0..w {
			let p = &data[(row * w + x) * 4..][..4];
			pixels.push(match (p[2], p[1], p[0], p[3]) {
				(0, 0, 0, 255) => Pixel::Tile,
				(0, 0, 255, 255) => Pixel::Neighbour(1, 0),
				(0, 255, 0, 255) => Pixel::Neighbour(-1, 0),
				(255, 0, 0, 255) => Pixel::Neighbour(-1, 1),
				(255, 255, 255, 255) => Pixel::Neighbour(1, -1),
				_ => Pixel::None,
			});
		}
	}
	Some((w, h, pixels))
}

/// Whether axial `(q, r)` is on the map (the original's own test).
fn on_map(g: &TacticalGridState, q: i64, r: i64) -> bool {
	q >= 0 && q < g.width && r >= -(q / 2) && r < g.height - q / 2
}

/// The point's offset from the top-left corner of tile `(q, r)`'s cell, in whole
/// pixels (truncated).
fn local(q: i64, r: i64, x: f32, y: f32) -> (i32, i32) {
	let lx = x - q as f32 * COL_W;
	let ly = y - (q as f32 * HALF_ROW + r as f32 * ROW_H);
	(lx as i32, ly as i32)
}

/// The level a tile is drawn at: its own, but no higher than the camera's.
fn shown_level(g: &TacticalGridState, q: i64, r: i64, camera_level: i64) -> usize {
	let id = g.idx_axial(q, r).expect("on the map");
	g.tiles[id].level.min(camera_level).clamp(0, 3) as usize
}

/// Whether the point lies on raised tile `(q, r)` as drawn (its top or its wall).
fn hits(g: &TacticalGridState, masks: &HexMasks, camera_level: i64, q: i64, r: i64, x: f32, y: f32) -> bool {
	if !on_map(g, q, r) {
		return false;
	}
	let (lx, ly) = local(q, r, x, y);
	let level = shown_level(g, q, r, camera_level);
	if lx < 0 || ly < 0 || lx >= TILE_W as i32 || ly as f32 >= level as f32 * LEVEL_H + ROW_H {
		return false;
	}
	masks.pixel(level, lx, ly) == Pixel::Tile
}

/// The point halfway between tiles `a` and `b` (their world positions, height
/// included), in f32 the way the original gets there: the direction normalized, then
/// scaled back by the distance and halved (disassembled, `getPosBetweenThisAnd`
/// 0x4db080 and `getTileBetweenThisAnd` 0x4db190).
pub(crate) fn midpoint(g: &TacticalGridState, a: usize, b: usize) -> (f32, f32) {
	let (ax, ay) = g.tiles[a].world_pos();
	let (bx, by) = g.tiles[b].world_pos();
	let (ax, ay) = (ax as f32, ay as f32);
	let (dx, dy) = (bx as f32 - ax, by as f32 - ay);
	let len = ((dx * dx + dy * dy) as f64).sqrt() as f32;
	let inv = 1.0f32 / len;
	(inv * dx * len * 0.5 + ax, inv * dy * len * 0.5 + ay)
}

/// `tile.getTileBetweenThisAnd(other)`: the tile picked ([`world_to_tile`]) at the
/// [`midpoint`] of the two; the tile itself if `other` is the same tile. The AI frames
/// its camera on it, and the kraken's tentacles check it.
pub(crate) fn tile_between(g: &TacticalGridState, masks: &HexMasks, camera_level: i64, a: usize, b: usize) -> usize {
	if a == b {
		return a;
	}
	let (x, y) = midpoint(g, a, b);
	let (q, r) = world_to_tile(g, masks, camera_level, x as f64, y as f64);
	g.idx_axial(q, r).expect("world_to_tile clamps onto the map")
}

/// `Tactical.worldToTile(pos)`: the axial coords of the tile at world point `(x, y)`,
/// clamped onto the map.
pub(crate) fn world_to_tile(g: &TacticalGridState, masks: &HexMasks, camera_level: i64, x: f64, y: f64) -> (i64, i64) {
	let (x, y) = (x as f32, y as f32);
	let mut q = (x / COL_W) as i64;
	let mut r = ((y - q as f32 * HALF_ROW) / ROW_H) as i64;
	if 0.0 > y - (q as f32 * HALF_ROW + r as f32 * ROW_H) {
		r -= 1;
	}
	if on_map(g, q, r) {
		// Raised tiles in front of the cell (lower rows first), as drawn over it.
		const IN_FRONT: [(i64, i64); 6] = [(0, -2), (1, -2), (-1, -1), (0, -1), (1, -1), (-1, 0)];
		match IN_FRONT.iter().find(|(dq, dr)| hits(g, masks, camera_level, q + dq, r + dr, x, y)) {
			Some((dq, dr)) => {
				q += dq;
				r += dr;
			}
			None => {
				let (lx, ly) = local(q, r, x, y);
				let level = shown_level(g, q, r, camera_level);
				if let Pixel::Neighbour(dq, dr) = masks.pixel(level, lx.max(0), ly) {
					q += dq;
					r += dr;
				}
			}
		}
	}
	let q = q.min(g.width - 1).max(0);
	let r = r.min(g.height - 1 - q / 2).max(-(q / 2));
	(q, r)
}
