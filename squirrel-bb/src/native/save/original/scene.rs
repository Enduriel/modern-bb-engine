//! The world scene as the original saves it (`0x55f7b0` writes, `0x55f160` reads): a
//! header, the tile grid, and the named layers that hold every drawn thing: the terrain
//! (`tiles`), the world's entities and decorations (`detail`, `entity`), and `effect`.
//!
//! Tiles (the grid's and the terrain layer's) are stored column by column: index
//! `x * height + y`.

use super::super::stream::{SaveReader, SaveWriter};
use super::record::{SceneEntity, type_id};
use super::wire;

const MAGIC: u32 = 0xb00b_b00b;
/// The scene layout version the load accepts.
const VERSION: u16 = 6;

/// A world tile's saved state (the grid's `0x578a30` and `WorldTile`'s `0x4fa450`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct WorldTile {
	/// Unnamed (`+0xc`, always 0).
	pub i0c: i8,
	/// bit0 IsDiscovered, bit1 (always set), bit2 IsOccupied.
	pub flags: u32,
	pub type_: u8,
	pub subtype: u8,
	/// The low byte of the region (the original keeps a `u16` and saves a byte).
	pub region: u8,
	pub subregion: u8,
	pub road_directions: u8,
	pub river_directions: u8,
	pub heat: u8,
	pub tactical_type: u8,
}

pub(crate) const TILE_DISCOVERED: u32 = 1;
pub(crate) const TILE_ALWAYS: u32 = 2;
pub(crate) const TILE_OCCUPIED: u32 = 4;

impl WorldTile {
	fn read(r: &mut SaveReader) -> Result<Self, String> {
		let i0c = r.u8()? as i8;
		let flags = r.u32()?;
		let b = r.raw(8)?;
		Ok(WorldTile {
			i0c,
			flags,
			type_: b[0],
			subtype: b[1],
			region: b[2],
			subregion: b[3],
			road_directions: b[4],
			river_directions: b[5],
			heat: b[6],
			tactical_type: b[7],
		})
	}

	fn write(&self, w: &mut SaveWriter) {
		w.u8(self.i0c as u8);
		w.u32(self.flags);
		w.buf.extend_from_slice(&[
			self.type_,
			self.subtype,
			self.region,
			self.subregion,
			self.road_directions,
			self.river_directions,
			self.heat,
			self.tactical_type,
		]);
	}
}

/// The scene's grid.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Grid {
	/// Unnamed (`+0x1c`, `+0x20`; (0, 2) in every save seen).
	pub origin: (i32, i32),
	pub width: i32,
	pub height: i32,
	/// Column by column.
	pub tiles: Vec<WorldTile>,
}

impl Grid {
	pub fn len(&self) -> usize {
		(self.width.max(0) * self.height.max(0)) as usize
	}
}

/// A terrain layer's tile: its brush and how it's drawn (`0x554220`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct DrawableTile {
	/// The brush's name hash; 0 for none.
	pub brush: u64,
	/// bit1: flipped (`WorldTile.IsFlipped`).
	pub flags: u8,
	pub b9: u8,
	pub color: [u8; 4],
}

pub(crate) const DRAWABLE_FLIPPED: u8 = 2;

impl DrawableTile {
	fn read(r: &mut SaveReader) -> Result<Self, String> {
		Ok(DrawableTile {
			brush: r.i64()? as u64,
			flags: r.u8()?,
			b9: r.u8()?,
			color: wire::bytes4(r)?,
		})
	}

	fn write(&self, w: &mut SaveWriter) {
		w.i64(self.brush as i64);
		w.u8(self.flags);
		w.u8(self.b9);
		w.buf.extend_from_slice(&self.color);
	}
}

/// The terrain layer (`HexTileLayer`, `0x557a40`; `HexTileQueuedLayer` has its layout).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct HexTileLayer {
	/// The hex metrics (240, 120, 400, 200, 2, 0 for the world).
	pub metrics: [f32; 6],
	pub i4c: i32,
	pub u5c: u32,
	/// The mask texture (`gfx/masks/hex_mask_world`).
	pub mask: Vec<u8>,
	/// The tiles (the scene grid's size, column by column) and the nested layers, when
	/// the layer's flags have bit 8.
	pub tiles: Option<(Vec<DrawableTile>, Vec<Layer>)>,
}

/// A quadtree of entities (`QuadTreeEntityLayer`, `0x55c650`). The entities are in the
/// tree's traversal order, which depends on the order they were added in: the original
/// doesn't keep it across a load.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct QuadTreeLayer {
	/// 8 (the tree's depth).
	pub depth: u8,
	/// x0, x1, y0, y1.
	pub bounds: [f32; 4],
	pub entities: Vec<SceneEntity>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum LayerBody {
	Tiles(HexTileLayer),
	QuadTree(QuadTreeLayer),
}

/// A layer: named for the scene's (`tiles`, `detail`, `entity`, `effect`), unnamed when
/// nested in a terrain layer.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Layer {
	/// `+0x2c` (0 for `tiles` and `detail`, 1 for `entity` and `effect`); nested layers
	/// have none.
	pub u2c: Option<u32>,
	pub name: Vec<u8>,
	pub type_: u64,
	pub flags: u32,
	/// Saved when the flags have bit 4.
	pub body: Option<LayerBody>,
}

const LAYER_SAVED: u32 = 4;
const LAYER_CONTENT: u32 = 8;

impl Layer {
	fn read_body(&mut self, r: &mut SaveReader, tiles: usize) -> Result<(), String> {
		if self.flags & LAYER_SAVED == 0 {
			return Ok(());
		}
		let body = if self.type_ == type_id("HexTileLayer") || self.type_ == type_id("HexTileQueuedLayer") {
			let mut metrics = [0.0; 6];
			for m in &mut metrics {
				*m = r.f32()?;
			}
			let mut layer = HexTileLayer {
				metrics,
				i4c: wire::i32(r)?,
				u5c: r.u32()?,
				mask: r.bytes()?,
				tiles: None,
			};
			if self.flags & LAYER_CONTENT != 0 {
				let drawn = (0..tiles).map(|_| DrawableTile::read(r)).collect::<Result<Vec<_>, _>>()?;
				let mut nested = Vec::new();
				for _ in 0..r.u8()? {
					let mut l = Layer {
						u2c: None,
						name: Vec::new(),
						type_: r.i64()? as u64,
						flags: r.u32()?,
						body: None,
					};
					l.read_body(r, tiles)?;
					nested.push(l);
				}
				layer.tiles = Some((drawn, nested));
			}
			LayerBody::Tiles(layer)
		} else if self.type_ == type_id("QuadTreeEntityLayer") {
			let depth = r.u8()?;
			let mut bounds = [0.0; 4];
			for b in &mut bounds {
				*b = r.f32()?;
			}
			let n = r.u32()?;
			let entities = (0..n).map(|_| SceneEntity::read(r)).collect::<Result<Vec<_>, _>>()?;
			LayerBody::QuadTree(QuadTreeLayer { depth, bounds, entities })
		} else {
			return Err(format!("a scene layer of an unknown class ({:#018x})", self.type_));
		};
		self.body = Some(body);
		Ok(())
	}

	fn write(&self, w: &mut SaveWriter) {
		if let Some(u2c) = self.u2c {
			w.u32(u2c);
			w.bytes(&self.name);
		}
		w.i64(self.type_ as i64);
		w.u32(self.flags);
		match &self.body {
			None => {}
			Some(LayerBody::Tiles(t)) => {
				for m in t.metrics {
					w.f32(m);
				}
				w.u32(t.i4c as u32);
				w.u32(t.u5c);
				w.bytes(&t.mask);
				if let Some((drawn, nested)) = &t.tiles {
					for d in drawn {
						d.write(w);
					}
					w.u8(nested.len() as u8);
					for l in nested {
						l.write(w);
					}
				}
			}
			Some(LayerBody::QuadTree(q)) => {
				w.u8(q.depth);
				for b in q.bounds {
					w.f32(b);
				}
				w.u32(q.entities.len() as u32);
				for e in &q.entities {
					e.write(w);
				}
			}
		}
	}

	/// The layer's entities, if it's a quadtree.
	pub fn entities(&self) -> &[SceneEntity] {
		match &self.body {
			Some(LayerBody::QuadTree(q)) => &q.entities,
			_ => &[],
		}
	}
}

/// The world scene.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Scene {
	/// Unnamed (`+0xd4`, 41).
	pub d4: u32,
	pub name: Vec<u8>,
	/// RGBA (`World.setAmbientLightingColor`).
	pub ambient_color: [u8; 4],
	pub ambient_saturation: f32,
	pub b8: u32,
	/// Written as 0, read and dropped.
	pub zeros: [u32; 3],
	/// Unnamed (`+0x28`).
	pub r28: [u8; 8],
	pub grid: Option<Grid>,
	/// The application data's id (1); the world has no application data.
	pub e0: u32,
	pub layers: Vec<Layer>,
}

impl Scene {
	pub fn read(r: &mut SaveReader) -> Result<Self, String> {
		if r.u32()? != MAGIC {
			return Err("Not a saved scene.".to_string());
		}
		let version = r.u16()?;
		if version != VERSION {
			return Err(format!("the saved scene's layout {version} isn't the original's {VERSION}"));
		}
		let mut s = Scene {
			d4: r.u32()?,
			name: r.bytes()?,
			ambient_color: wire::bytes4(r)?,
			ambient_saturation: r.f32()?,
			b8: r.u32()?,
			zeros: [r.u32()?, r.u32()?, r.u32()?],
			r28: r.raw(8)?.try_into().expect("8 bytes"),
			grid: None,
			e0: 0,
			layers: Vec::new(),
		};
		if r.bool()? {
			let origin = (wire::i32(r)?, wire::i32(r)?);
			let (width, height) = (wire::i32(r)?, wire::i32(r)?);
			let mut grid = Grid {
				origin,
				width,
				height,
				tiles: Vec::new(),
			};
			grid.tiles = (0..grid.len()).map(|_| WorldTile::read(r)).collect::<Result<Vec<_>, _>>()?;
			s.grid = Some(grid);
		}
		s.e0 = r.u32()?;
		if r.u16()? != 0 {
			return Err("a saved scene with free scene objects (not in the world's saves)".to_string());
		}
		let tiles = s.grid.as_ref().map_or(0, Grid::len);
		for _ in 0..r.u8()? {
			let mut l = Layer {
				u2c: Some(r.u32()?),
				name: r.bytes()?,
				type_: r.i64()? as u64,
				flags: r.u32()?,
				body: None,
			};
			l.read_body(r, tiles)?;
			s.layers.push(l);
		}
		if r.u32()? != 0 {
			return Err("the saved scene doesn't end where it should".to_string());
		}
		Ok(s)
	}

	pub fn write(&self, w: &mut SaveWriter) {
		w.u32(MAGIC);
		w.u16(VERSION);
		w.u32(self.d4);
		w.bytes(&self.name);
		w.buf.extend_from_slice(&self.ambient_color);
		w.f32(self.ambient_saturation);
		w.u32(self.b8);
		for z in self.zeros {
			w.u32(z);
		}
		w.buf.extend_from_slice(&self.r28);
		w.bool(self.grid.is_some());
		if let Some(g) = &self.grid {
			w.u32(g.origin.0 as u32);
			w.u32(g.origin.1 as u32);
			w.u32(g.width as u32);
			w.u32(g.height as u32);
			for t in &g.tiles {
				t.write(w);
			}
		}
		w.u32(self.e0);
		w.u16(0);
		w.u8(self.layers.len() as u8);
		for l in &self.layers {
			l.write(w);
		}
		w.u32(0);
	}

	/// The named layer `name`.
	pub fn layer(&self, name: &str) -> Option<&Layer> {
		self.layers.iter().find(|l| l.name == name.as_bytes())
	}
}
