//! [`FogMask`]: how much fog of war covers each part of the world map.
//!
//! Which tiles count as discovered and where the fog is drawn are separate in the
//! original. `uncoverFogOfWar(pos, radius)` (`0x4ef3f0`) marks tiles discovered by
//! hex distance, and also stamps the `fog_of_war_mask` brush into a 2048×2048 render
//! target that spans the whole map (the fog object at `World+0x28`, stamp `0x4a2370`,
//! disassembled 2026-10-14):
//! - the target is cleared to opaque black (`0x4a22d0`) when the map is made, and is
//!   saved with the game ("Corrupt data before FoW");
//! - its texels map onto the map's extent, from the origin to the centre of the tile
//!   past the last corner, `(width, height - width / 2)` in axial coordinates, each
//!   axis truncated to an integer (`0x4f4ce7`, `0x4a2200`);
//! - the brush is drawn at `pos` with scale `radius / 100`, with blend equation
//!   `GL_MIN`, so every texel keeps the lowest alpha stamped into it;
//! - the fog pass (`0x4a24c0`) draws `gfx/fow.png` over the screen with this mask's
//!   alpha as its opacity.
//!
//! The brush is opaque but for a soft clear hole about 0.95 of its half-size across,
//! so a stamp clears a disc of about `radius` world units. Discovery reaches further
//! (`trunc(radius / 100) + 3` tiles of 120 units), so discovered tiles at the edge
//! still show fog.
//!
//! Only alpha matters to the fog pass, so the mask keeps one byte per texel, rows
//! from the map's bottom (y up), as the render target's are. The brush's picture comes
//! from the host ([`FogMask::set_stamp`]); without one, uncovering leaves the mask as
//! it is.
//!
//! Divergence (the world map can be any size; see `docs/differences.md`): the
//! original's mask is always 2048². Ours is 2048² up to the vanilla map's extent and
//! grows with a larger map, so a texel covers no more of the world than at vanilla size
//! ([`mask_size_for`]).

use super::save::stream::{SaveReader, SaveWriter};

/// Texels along each side of the original's mask, and of ours up to the vanilla map's
/// extent.
pub const MASK_SIZE: usize = 2048;

/// The extent of the vanilla map (140 × 170 tiles, as `fog_extent` takes it), which a
/// 2048² mask spans.
fn vanilla_extent() -> (f32, f32) {
	let (x, y) = super::world_layout::tile_pos(140, 170);
	(x.trunc() as f32, y.trunc() as f32)
}

/// The largest mask: a GPU texture side every adapter takes.
const MAX_MASK_SIZE: usize = 8192;

/// The side of a mask spanning `world`: 2048 up to the vanilla extent, and beyond it as
/// many texels as keep a texel's world size (in 256s), up to [`MAX_MASK_SIZE`].
pub fn mask_size_for(world: (f32, f32)) -> usize {
	let vanilla = vanilla_extent();
	let scale = (world.0 / vanilla.0).max(world.1 / vanilla.1);
	if !(scale > 1.0) {
		return MASK_SIZE;
	}
	let texels = (MASK_SIZE as f32 * scale / 256.0).ceil() as usize * 256;
	texels.clamp(MASK_SIZE, MAX_MASK_SIZE)
}

/// The brush stamped into the mask: its frame, and the alpha of the atlas it sits in.
#[derive(Clone, Debug)]
pub struct FogStamp {
	/// The atlas's alpha channel, one byte per texel, rows as stored.
	pub alpha: Vec<u8>,
	pub width: u32,
	pub height: u32,
	/// The frame's atlas coordinates: left, right, and the `v` at the rect's low and
	/// high y edges (as a brush frame gives them).
	pub uv: [f32; 4],
	/// The frame around the anchor in sprite pixels, y up: left, right, bottom, top,
	/// the sprite's offset included.
	pub rect: [f32; 4],
}

impl FogStamp {
	/// Bilinear alpha at atlas coordinates `(u, v)`, clamped to the frame.
	fn sample(&self, u: f32, v: f32) -> f32 {
		let (w, h) = (self.width as f32, self.height as f32);
		let [u0, u1, v0, v1] = self.uv;
		let clamp = |x: f32, a: f32, b: f32, size: f32| {
			let (lo, hi) = (a.min(b) * size, a.max(b) * size);
			x.clamp(lo + 0.5, (hi - 0.5).max(lo + 0.5)) - 0.5
		};
		let x = clamp(u * w, u0, u1, w);
		let y = clamp(v * h, v0, v1, h);
		let (x0, y0) = (x.floor(), y.floor());
		let (fx, fy) = (x - x0, y - y0);
		let at = |x: f32, y: f32| {
			let (x, y) = ((x as usize).min(self.width as usize - 1), (y as usize).min(self.height as usize - 1));
			f32::from(self.alpha[y * self.width as usize + x])
		};
		let top = at(x0, y0) * (1.0 - fx) + at(x0 + 1.0, y0) * fx;
		let bottom = at(x0, y0 + 1.0) * (1.0 - fx) + at(x0 + 1.0, y0 + 1.0) * fx;
		top * (1.0 - fy) + bottom * fy
	}
}

/// A changed rectangle of the mask, for a renderer to upload.
#[derive(Clone, Debug, PartialEq)]
pub struct FogPatch {
	/// The whole mask's side, in texels (see [`mask_size_for`]).
	pub mask: u32,
	/// The first texel column and row (rows from the map's bottom).
	pub x: u32,
	pub y: u32,
	pub width: u32,
	pub height: u32,
	/// Alpha, row by row.
	pub alpha: Vec<u8>,
}

/// A texel rectangle: columns `x0..x1`, rows `y0..y1`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Rect {
	x0: usize,
	y0: usize,
	x1: usize,
	y1: usize,
}

impl Rect {
	fn full(size: usize) -> Rect {
		Rect {
			x0: 0,
			y0: 0,
			x1: size,
			y1: size,
		}
	}

	fn union(self, other: Rect) -> Rect {
		Rect {
			x0: self.x0.min(other.x0),
			y0: self.y0.min(other.y0),
			x1: self.x1.max(other.x1),
			y1: self.y1.max(other.y1),
		}
	}
}

/// The fog-of-war mask (see the module docs).
#[derive(Default)]
pub struct FogMask {
	/// `size²` alpha bytes once the map exists; empty before.
	alpha: Vec<u8>,
	/// Texels along each side (see [`mask_size_for`]).
	size: usize,
	/// The map's extent in world units that the mask spans.
	world: (f32, f32),
	stamp: Option<FogStamp>,
	/// Changed since a renderer last took a patch.
	dirty: Option<Rect>,
	/// The last stamp's position and scale: stamping it again changes nothing.
	last: Option<(f32, f32, f32)>,
}

impl FogMask {
	/// Fully fogged, spanning a map that reaches `world` (world units).
	pub fn reset(&mut self, world: (f32, f32)) {
		self.size = mask_size_for(world);
		self.alpha.clear();
		self.alpha.resize(self.size * self.size, 255);
		self.world = world;
		self.dirty = Some(Rect::full(self.size));
		self.last = None;
	}

	/// Texels along each side (0 before the map exists).
	#[cfg(test)]
	pub fn size(&self) -> usize {
		self.size
	}

	/// The brush that uncovering stamps.
	pub fn set_stamp(&mut self, stamp: FogStamp) {
		self.stamp = Some(stamp);
		self.last = None;
	}

	/// The map extent the mask spans, in world units.
	#[cfg(test)]
	pub fn world_size(&self) -> (f32, f32) {
		self.world
	}

	/// Alpha at texel `(x, y)` (rows from the bottom), 255 before the map exists.
	#[cfg(test)]
	pub fn at(&self, x: usize, y: usize) -> u8 {
		self.alpha.get(y * self.size + x).copied().unwrap_or(255)
	}

	/// Stamp the brush at world position `pos`, scaled by `scale`.
	pub fn stamp(&mut self, pos: (f32, f32), scale: f32) {
		let Some(stamp) = &self.stamp else { return };
		if self.alpha.is_empty() || self.last == Some((pos.0, pos.1, scale)) || !(scale > 0.0) {
			return;
		}
		self.last = Some((pos.0, pos.1, scale));
		let (n, size) = (self.size, self.size as f32);
		let (kx, ky) = (size / self.world.0, size / self.world.1);
		let [left, right, bottom, top] = stamp.rect;
		let (x0, x1) = (pos.0 + left * scale, pos.0 + right * scale);
		let (y0, y1) = (pos.1 + bottom * scale, pos.1 + top * scale);
		// The texels whose centres the quad covers.
		let first = |edge: f32, k: f32| ((edge * k - 0.5).ceil().max(0.0) as usize).min(n);
		let (i0, i1) = (first(x0, kx), first(x1, kx));
		let (j0, j1) = (first(y0, ky), first(y1, ky));
		if i0 >= i1 || j0 >= j1 {
			return;
		}
		let [u0, u1, v0, v1] = stamp.uv;
		for j in j0..j1 {
			let wy = (j as f32 + 0.5) / ky;
			let v = v0 + (wy - y0) / (y1 - y0) * (v1 - v0);
			let row = &mut self.alpha[j * n..(j + 1) * n];
			for (i, texel) in row.iter_mut().enumerate().take(i1).skip(i0) {
				if *texel == 0 {
					continue;
				}
				let wx = (i as f32 + 0.5) / kx;
				let u = u0 + (wx - x0) / (x1 - x0) * (u1 - u0);
				let a = stamp.sample(u, v).round() as u8;
				*texel = (*texel).min(a);
			}
		}
		let changed = Rect {
			x0: i0,
			y0: j0,
			x1: i1,
			y1: j1,
		};
		self.dirty = Some(self.dirty.map_or(changed, |d| d.union(changed)));
	}

	/// Whether every texel a pass sampling the mask over the world rectangle `[left,
	/// right, bottom, top]` reads is clear (alpha 0): those under it, their neighbours
	/// that bilinear filtering blends in, and past the map's edges the edge texels (the
	/// fog pass clamps). False before the map exists.
	pub fn clear_within(&self, [left, right, bottom, top]: [f32; 4]) -> bool {
		let n = self.size;
		if self.alpha.is_empty() || n == 0 || !(self.world.0 > 0.0 && self.world.1 > 0.0) {
			return false;
		}
		let (kx, ky) = (n as f32 / self.world.0, n as f32 / self.world.1);
		// A sample at texel coordinate t reads texels floor(t - 0.5) and the next.
		let span = |lo: f32, hi: f32, k: f32| {
			let last = n as f32 - 1.0;
			let first = (lo * k - 0.5).floor().clamp(0.0, last) as usize;
			let end = ((hi * k - 0.5).floor() + 1.0).clamp(0.0, last) as usize;
			(first, end)
		};
		let ((i0, i1), (j0, j1)) = (span(left, right, kx), span(bottom, top, ky));
		(j0..=j1).all(|j| self.alpha[j * n + i0..=j * n + i1].iter().all(|&a| a == 0))
	}

	/// What changed since the last call, or `None` if nothing did.
	pub fn take_patch(&mut self) -> Option<FogPatch> {
		let r = self.dirty.take()?;
		if self.alpha.is_empty() {
			return None;
		}
		let n = self.size;
		let mut alpha = Vec::with_capacity((r.x1 - r.x0) * (r.y1 - r.y0));
		for j in r.y0..r.y1 {
			alpha.extend_from_slice(&self.alpha[j * n + r.x0..j * n + r.x1]);
		}
		Some(FogPatch {
			mask: n as u32,
			x: r.x0 as u32,
			y: r.y0 as u32,
			width: (r.x1 - r.x0) as u32,
			height: (r.y1 - r.y0) as u32,
			alpha,
		})
	}

	/// Replace the mask (`size²` bytes; the original saves a vanilla map's, 2048²).
	pub(crate) fn set_alpha(&mut self, alpha: Vec<u8>) -> Result<(), String> {
		let n = self.size;
		if alpha.len() != n * n {
			return Err(format!("a fog of war mask of {} bytes (not {n}²)", alpha.len()));
		}
		self.alpha = alpha;
		self.dirty = Some(Rect::full(n));
		self.last = None;
		Ok(())
	}

	/// The mask's bytes (`size²`, or none before the map exists).
	pub(crate) fn alpha(&self) -> &[u8] {
		&self.alpha
	}

	/// Write the mask as runs of equal bytes.
	pub(crate) fn save(&self, w: &mut SaveWriter) {
		w.f32(self.world.0);
		w.f32(self.world.1);
		let runs = runs(&self.alpha);
		w.u32(runs.len() as u32);
		for (value, len) in runs {
			w.u8(value);
			w.u32(len);
		}
	}

	/// Read a mask [`save`](Self::save) wrote. Its size follows from the extent saved
	/// with it, as when the map was made.
	pub(crate) fn load(&mut self, r: &mut SaveReader) -> Result<(), String> {
		let world = (r.f32()?, r.f32()?);
		let n = mask_size_for(world);
		let count = r.u32()?;
		let mut alpha = Vec::new();
		for _ in 0..count {
			let (value, len) = (r.u8()?, r.u32()? as usize);
			if alpha.len() + len > n * n {
				return Err("fog of war mask: too much data".into());
			}
			alpha.resize(alpha.len() + len, value);
		}
		if !alpha.is_empty() && alpha.len() != n * n {
			return Err("fog of war mask: truncated".into());
		}
		self.alpha = alpha;
		self.size = if self.alpha.is_empty() { 0 } else { n };
		self.world = world;
		self.dirty = Some(Rect::full(n));
		self.last = None;
		Ok(())
	}
}

/// `bytes` as (value, length) runs.
fn runs(bytes: &[u8]) -> Vec<(u8, u32)> {
	let mut out: Vec<(u8, u32)> = Vec::new();
	for &b in bytes {
		match out.last_mut() {
			Some((value, len)) if *value == b => *len += 1,
			_ => out.push((b, 1)),
		}
	}
	out
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A 4×4 atlas whose 2×2 middle is clear, framed 8×8 sprite pixels around the
	/// anchor.
	fn stamp() -> FogStamp {
		#[rustfmt::skip]
		let alpha = vec![
			255, 255, 255, 255,
			255,   0,   0, 255,
			255,   0,   0, 255,
			255, 255, 255, 255,
		];
		FogStamp {
			alpha,
			width: 4,
			height: 4,
			uv: [0.0, 1.0, 0.0, 1.0],
			rect: [-4.0, 4.0, -4.0, 4.0],
		}
	}

	/// A mask over a 2048×2048-unit map: one texel per world unit.
	fn mask() -> FogMask {
		let mut m = FogMask::default();
		m.reset((2048.0, 2048.0));
		m.set_stamp(stamp());
		m
	}

	#[test]
	fn a_stamp_clears_its_hole_and_keeps_its_rim() {
		let mut m = mask();
		m.stamp((100.0, 200.0), 10.0);
		// The 80×80 quad spans 60..140 × 160..240; its clear middle is the inner half.
		assert_eq!(m.at(100, 200), 0);
		assert_eq!(m.at(61, 161), 255, "the brush's opaque rim");
		assert_eq!(m.at(59, 200), 255, "outside the quad");
		assert_eq!(m.at(100, 241), 255);
	}

	#[test]
	fn a_rectangle_is_clear_only_where_every_texel_read_there_is() {
		let mut m = FogMask::default();
		assert!(!m.clear_within([0.0, 10.0, 0.0, 10.0]), "no map yet");
		m = mask();
		m.stamp((100.0, 200.0), 10.0);
		// The brush is sampled bilinearly: texels 90..110 (across and up) are clear,
		// fading to opaque from there to the quad's edges.
		assert_eq!((m.at(89, 200), m.at(90, 200), m.at(109, 200), m.at(110, 200)), (6, 0, 0, 6));
		assert!(m.clear_within([91.0, 109.0, 191.0, 209.0]));
		// A sample at x = 90.2 blends texel 89 in.
		assert!(!m.clear_within([90.2, 109.0, 191.0, 209.0]));
		assert!(!m.clear_within([91.0, 109.0, 191.0, 230.0]), "reaches the rim above");
		// Past the map's edge the pass reads the edge texels, which are fogged.
		assert!(!m.clear_within([-50.0, -10.0, 185.0, 215.0]));
	}

	#[test]
	fn stamps_keep_the_lowest_alpha() {
		let mut m = mask();
		m.stamp((100.0, 100.0), 10.0);
		// Overlapping the first hole with a second stamp's opaque rim keeps it clear.
		m.stamp((135.0, 100.0), 10.0);
		assert_eq!(m.at(100, 100), 0);
		assert_eq!(m.at(135, 100), 0);
	}

	#[test]
	fn a_larger_map_gets_a_larger_mask() {
		let vanilla = vanilla_extent();
		assert_eq!(mask_size_for(vanilla), MASK_SIZE, "vanilla keeps the original's 2048²");
		assert_eq!(mask_size_for((vanilla.0 / 2.0, vanilla.1)), MASK_SIZE);
		assert_eq!(mask_size_for((vanilla.0 * 2.0, vanilla.1 * 1.65)), 4096);
		assert_eq!(mask_size_for((vanilla.0 * 1.01, vanilla.1)), 2304, "in 256s");
		assert_eq!(mask_size_for((vanilla.0 * 10.0, vanilla.1)), 8192, "at most a texture's side");
		let mut mask = FogMask::default();
		mask.reset((vanilla.0 * 2.0, vanilla.1 * 1.65));
		assert_eq!(mask.size(), 4096);
		let patch = mask.take_patch().expect("the new mask");
		assert_eq!((patch.mask, patch.width, patch.height), (4096, 4096, 4096));
	}

	#[test]
	fn the_mask_spans_the_map_extent() {
		let mut m = FogMask::default();
		// Ten world units per texel across, five up.
		m.reset((20480.0, 10240.0));
		m.set_stamp(stamp());
		m.stamp((5000.0, 2500.0), 10.0);
		assert_eq!(m.at(500, 500), 0);
		// The quad is 80 units: texels 496..504 across, 492..508 up.
		assert_eq!(m.at(499, 500), 0);
		assert_eq!(m.at(500, 492), 255, "the rim");
		assert_eq!(m.at(500, 491), 255, "outside");
		let patch = m.take_patch().unwrap();
		assert_eq!((patch.x, patch.y, patch.width, patch.height), (0, 0, 2048, 2048));
		m.stamp((5000.0, 2600.0), 10.0);
		let patch = m.take_patch().unwrap();
		assert_eq!((patch.x, patch.y, patch.width, patch.height), (496, 512, 8, 16));
	}

	#[test]
	fn without_a_stamp_or_a_map_nothing_changes() {
		let mut m = FogMask::default();
		m.set_stamp(stamp());
		m.stamp((10.0, 10.0), 1.0);
		assert!(m.take_patch().is_none());
		let mut m = FogMask::default();
		m.reset((2048.0, 2048.0));
		m.take_patch();
		m.stamp((100.0, 100.0), 10.0);
		assert_eq!(m.at(100, 100), 255);
		assert!(m.take_patch().is_none());
	}

	#[test]
	fn patches_carry_what_changed() {
		let mut m = mask();
		let full = m.take_patch().expect("a new mask uploads whole");
		assert_eq!((full.width, full.height), (2048, 2048));
		assert!(m.take_patch().is_none());
		m.stamp((100.0, 200.0), 10.0);
		let patch = m.take_patch().expect("the stamp's texels");
		assert_eq!((patch.x, patch.y, patch.width, patch.height), (60, 160, 80, 80));
		assert_eq!(patch.alpha[40 * 80 + 40], 0);
		m.stamp((100.0, 200.0), 10.0);
		assert!(m.take_patch().is_none(), "the same stamp again changes nothing");
	}

	#[test]
	fn the_mask_saves_and_loads() {
		let mut m = mask();
		m.stamp((300.0, 400.0), 7.0);
		let mut w = SaveWriter::default();
		m.save(&mut w);
		assert!(w.buf.len() < 64 * 1024, "runs keep it small: {}", w.buf.len());
		let mut loaded = FogMask::default();
		loaded.load(&mut SaveReader::new(w.buf)).unwrap();
		assert_eq!(loaded.world_size(), (2048.0, 2048.0));
		assert_eq!(loaded.alpha, m.alpha);
		assert_eq!(loaded.take_patch().map(|p| p.width), Some(2048));
	}
}
