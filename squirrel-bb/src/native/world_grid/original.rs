//! The map's share of the original's save layout (see `save::original`): the tiles, the
//! terrain layer's brushes, the region names and the fog of war.

use super::{TileData, WorldGrid, bound_camera, fog_extent};
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
		t.brush = brush_name(sprites, d.brush);
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
		bound_camera(&g);
		Ok(())
	}

	/// The fog of war's mask as saved.
	pub(crate) fn load_original_fog(&self, mask: Vec<u8>) -> Result<(), String> {
		self.grid.borrow_mut().fog.set_alpha(mask)
	}
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
				text: t.text.clone(),
				scale: t.size / REGION_TEXT_SIZE,
				rotation: e.head.rotation,
				pos: e.head.pos,
			}),
			_ => None,
		})
		.collect()
}
