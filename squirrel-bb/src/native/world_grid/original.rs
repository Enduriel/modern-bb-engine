//! The map's share of the original's save layout (see `save::original`): the tiles, the
//! terrain layer's brushes, the decorations, the region names and the fog of war.
//!
//! A saved decoration (`WorldDecal`) joins the tile that contains its position, as the
//! original's load attaches it (`0x4e4490`), with its id, position and draw order kept.
//! A detail spawned with an offset into a neighbouring tile (the worldgen's straits)
//! therefore moves to that tile across a load, as in the original.

use std::cell::RefCell;
use std::rc::Rc;

use super::{GridState, TileData, WorldGrid, bound_camera, fog_extent};
use crate::native::save::original::record::{Head, SceneEntity, Text};
use crate::native::save::original::scene::{Grid, HexTileLayer, TILE_ALWAYS};
use crate::native::save::original::sprite::{ALWAYS, BRUSH_BITS, BrushLook, FLIPPED, IGNORE_AMBIENT_COLOR, Sprite, VISIBLE};
use crate::native::sprite_color::SpriteColor;
use crate::native::world_detail::{self, DetailData};
use crate::native::world_entities::{argb, rgba};
use crate::native::world_layout;
use crate::native::Sprites;
use crate::native::save::original::record::{Body, type_id};
use crate::native::save::original::scene::{
	DRAWABLE_FLIPPED, DrawableTile, LayerBody, Scene, TILE_DISCOVERED, TILE_OCCUPIED, WorldTile,
};
use crate::scene::SceneRegionText;

/// The original stores a region name's size as its scale times this (`0x4f0549`).
const REGION_TEXT_SIZE: f32 = 0.4;

/// The name of the brush with name hash `hash` ("" for none or an unknown one).
pub(crate) fn brush_name(sprites: &Sprites, hash: u64) -> String {
	if hash == 0 {
		return String::new();
	}
	sprites.with(|r| r.find_hash(hash).map(|id| r.sprite(id).name.clone())).unwrap_or_default()
}

fn tile(id: usize, col: i64, row: i64, saved: &WorldTile, drawn: Option<&DrawableTile>, sprites: &Sprites) -> TileData {
	let mut t = TileData::new(id, col, row);
	t.type_ = saved.type_.into();
	t.region = saved.region.into();
	t.subregion = saved.subregion.into();
	t.road_directions = saved.road_directions.into();
	t.river_directions = saved.river_directions.into();
	t.has_river = saved.river_directions != 0;
	t.heat = saved.heat.into();
	t.tactical_type = saved.tactical_type.into();
	t.is_discovered = saved.flags & TILE_DISCOVERED != 0;
	t.is_occupied = saved.flags & TILE_OCCUPIED != 0;
	if let Some(d) = drawn {
		t.brush = sprites.name(&brush_name(sprites, d.brush));
		t.is_flipped = d.flags & DRAWABLE_FLIPPED != 0;
	}
	t
}

impl WorldGrid {
	/// The map as saved in `scene`: its size, tiles, terrain and region names. The fog
	/// starts fully fogged (the save's mask comes last; [`Self::load_original_fog`]).
	pub(crate) fn load_original(&self, scene: &Scene) -> Result<(), String> {
		let mut g = self.grid.borrow_mut();
		let Some(grid) = &scene.grid else {
			return Err("the saved world has no map".to_string());
		};
		let (w, h) = (i64::from(grid.width), i64::from(grid.height));
		g.redrawn();
		let drawn = scene.layer("tiles").and_then(|l| match &l.body {
			Some(LayerBody::Tiles(t)) => t.tiles.as_ref().map(|(d, _)| d),
			_ => None,
		});
		// Saved column by column; ours row by row.
		let sprites = g.sprites.clone();
		g.tiles = (0..w * h)
			.map(|i| {
				let (col, row) = (i % w, i / w);
				let saved = (col * h + row) as usize;
				tile(i as usize, col, row, &grid.tiles[saved], drawn.and_then(|d| d.get(saved)), &sprites)
			})
			.collect();
		g.width = w;
		g.height = h;
		g.fog.reset(fog_extent(w, h));
		g.region_texts = region_texts(scene);
		let decal = type_id("WorldDecal");
		for e in scene.layers.iter().flat_map(|l| l.entities()).filter(|e| e.type_ == decal) {
			add_decal(&mut g, e);
		}
		bound_camera(&g);
		Ok(())
	}

	/// The fog of war's mask as saved.
	pub(crate) fn load_original_fog(&self, mask: Vec<u8>) -> Result<(), String> {
		self.grid.borrow_mut().fog.set_alpha(mask)
	}

	/// The map as the original saves it: the grid, the terrain layer, and the scene
	/// entities of the `detail` layer that are the map's (decorations, region names).
	pub(crate) fn original_map(&self) -> (Grid, HexTileLayer, Vec<SceneEntity>) {
		let g = self.grid.borrow();
		let (w, h) = (g.width, g.height);
		let mut tiles = Vec::with_capacity((w * h).max(0) as usize);
		let mut drawn = Vec::with_capacity(tiles.capacity());
		// Column by column.
		for col in 0..w {
			for row in 0..h {
				let t = &g.tiles[(row * w + col) as usize];
				let mut flags = TILE_ALWAYS;
				if t.is_discovered {
					flags |= TILE_DISCOVERED;
				}
				if t.is_occupied {
					flags |= TILE_OCCUPIED;
				}
				tiles.push(WorldTile {
					i0c: 0,
					flags,
					type_: t.type_ as u8,
					subtype: 0,
					region: t.region as u8,
					subregion: t.subregion as u8,
					road_directions: t.road_directions as u8,
					river_directions: t.river_directions as u8,
					heat: t.heat as u8,
					tactical_type: t.tactical_type as u8,
				});
				drawn.push(DrawableTile {
					brush: g.sprites.with(|r| BrushLook::of(r, &t.brush).hash),
					flags: if t.is_flipped { DRAWABLE_FLIPPED } else { 0 },
					b9: 0,
					color: [0xff; 4],
				});
			}
		}
		let grid = Grid {
			origin: (0, 2),
			width: w as i32,
			height: h as i32,
			tiles,
		};
		let layer = HexTileLayer {
			metrics: [240.0, 120.0, 400.0, 200.0, 2.0, 0.0],
			i4c: 0,
			u5c: 0x420,
			mask: b"gfx/masks/hex_mask_world".to_vec(),
			tiles: Some((drawn, Vec::new())),
		};
		let mut entities: Vec<SceneEntity> = g.tiles.iter().flat_map(|t| &t.details).map(|d| decal(&g, &d.borrow())).collect();
		entities.extend(g.region_texts.iter().map(region_text));
		(grid, layer, entities)
	}

	/// The fog of war's mask as the original saves it.
	pub(crate) fn original_fog(&self) -> Vec<u8> {
		self.grid.borrow().fog.alpha().to_vec()
	}
}

/// A decoration as the original saves it (a `WorldDecal`, see `spawnDetail`).
fn decal(g: &GridState, d: &DetailData) -> SceneEntity {
	let look = g.sprites.with(|r| BrushLook::of(r, &d.brush));
	let mut flags = ALWAYS | d.brush_bits;
	if d.visible {
		flags |= VISIBLE;
	}
	if d.flipped {
		flags |= FLIPPED;
	}
	if d.ignore_ambient {
		flags |= IGNORE_AMBIENT_COLOR;
	}
	SceneEntity {
		type_: type_id("WorldDecal"),
		head: Head {
			id: d.id as u32,
			pos: d.pos,
			z: d.z,
			i14: 0,
			rotation: 0.0,
			flags: 0x502c7d,
			u34: d.flags as u32,
			children: Vec::new(),
		},
		body: Body::Sprite(Sprite {
			brush: look.hash,
			flags,
			color: rgba(d.color.color),
			scale: d.scale as f32,
			rotation: d.rotation as f32,
			saturation: d.saturation as f32,
			f64_: 0.1,
			i58: 0,
			v4c: look.v4c,
		}),
	}
}

/// A region name as the original saves it (a `TextEntity`, see `spawnRegionText`).
fn region_text(t: &SceneRegionText) -> SceneEntity {
	SceneEntity {
		type_: type_id("TextEntity"),
		head: Head {
			id: t.id as u32,
			pos: t.pos,
			z: 0,
			i14: 0,
			rotation: t.rotation,
			flags: 0x503c7d,
			u34: 0,
			children: Vec::new(),
		},
		body: Body::Text(Text {
			font: b"gfx/fonts/cinzel_bold_100.fnt".to_vec(),
			offset: (0.0, 0.0),
			size: t.scale * REGION_TEXT_SIZE,
			alpha: 1.0,
			valign: 2,
			halign: 2,
			color: 0x66ff_ffff,
			color2: 0xffff_ffff,
			b9c: false,
			text: t.text.clone(),
		}),
	}
}

/// A saved decoration, on the tile containing it.
fn add_decal(g: &mut GridState, e: &SceneEntity) {
	let Body::Sprite(sprite) = &e.body else { return };
	let (x, y) = (f64::from(e.head.pos.0), f64::from(e.head.pos.1));
	let (q, r) = world_layout::world_to_tile(x, y, g.width, g.height);
	let Some(idx) = g.idx(q, r + q / 2) else { return };
	let brush = g.sprites.resolve(&brush_name(&g.sprites, sprite.brush));
	let (depth_bias, rotation_speed) = g
		.sprites
		.with(|reg| reg.get(&brush).map_or((0, 0.0), |(_, s)| (i64::from(s.depth_bias), s.rotation_speed)));
	let (cx, cy) = world_layout::tile_pos(q, r + q / 2);
	let mut data = DetailData {
		brush: g.sprites.name(&brush),
		// The z level it was spawned with, as far as its containing tile tells.
		z_level: i64::from(e.head.z) - y as i64 - depth_bias + (r & 1),
		flags: i64::from(e.head.u34),
		flipped: sprite.flags & FLIPPED != 0,
		visible: sprite.flags & VISIBLE != 0,
		color: SpriteColor::new(argb(sprite.color)),
		rotation: sprite.rotation.into(),
		saturation: sprite.saturation.into(),
		scale: sprite.scale.into(),
		ignore_ambient: sprite.flags & IGNORE_AMBIENT_COLOR != 0,
		id: e.head.id.into(),
		pos: e.head.pos,
		offset: (e.head.pos.0 - cx as f32, e.head.pos.1 - cy as f32),
		z: e.head.z,
		brush_bits: sprite.flags & BRUSH_BITS,
		..DetailData::default()
	};
	// The brush's rotation speed (torque isn't saved).
	data.spin.torque = rotation_speed;
	if g.ids.peek() <= data.id {
		g.ids.restore(data.id + 1);
	}
	let detail = Rc::new(RefCell::new(data));
	world_detail::note_spin(&g.spinning, &detail);
	g.tiles[idx].details.push(detail);
}

/// The region names: the map's text entities, in the order the layers keep them.
fn region_texts(scene: &Scene) -> Vec<SceneRegionText> {
	let text = type_id("TextEntity");
	scene
		.layers
		.iter()
		.flat_map(|l| l.entities())
		.filter(|e| e.type_ == text)
		.filter_map(|e| match &e.body {
			Body::Text(t) => Some(SceneRegionText {
				id: e.head.id.into(),
				text: t.text.clone(),
				scale: t.size / REGION_TEXT_SIZE,
				rotation: e.head.rotation,
				pos: e.head.pos,
			}),
			_ => None,
		})
		.collect()
}
