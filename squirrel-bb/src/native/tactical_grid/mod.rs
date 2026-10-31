//! The battle map: `Tactical.resizeScene`, `getTileSquare`/`getTile`, the tactical tiles
//! (see [`tile`]) and their details.
//!
//! The layout matches the original's (probed 2026-10-13): columns of hexes, square
//! coords `(col, row)`, axial coords `(q, r) = (col, row - col / 2)`, the same six
//! neighbour directions as the world map, and tile centres at
//! `(80 + 120·col, 40 + 80·row (+40 for odd columns))` pixels.
//!
//! Host-owned like the other services: tiles keep script references (their occupant,
//! `Properties`, `Items`), so [`clear`](TacticalGrid::clear) it before the VM closes.

pub(crate) mod camera;
pub(crate) mod detail;
pub(crate) mod faction_set;
pub(crate) mod heads;
mod heat;
mod memory;
mod off_map;
mod picking;
mod camera_callbacks;
mod orientation;
mod tile;
#[cfg(test)]
mod vis_ground_truth;
pub(crate) mod visibility;
mod walk;
pub(crate) mod range;
pub(crate) mod query;
pub(crate) mod zone;

use std::cell::RefCell;
use std::rc::Rc;

use squirrel::{Number, Retained, SQVM, SqClass, SqStrongRef, SqTable};

use crate::module::FileReader;
use crate::native::moving_sprites::Moving;
use crate::native::sprite_class::SpriteState;
use crate::native::{EngineRng, Sprites};

use super::overload::{make_dispatcher, overload_key};

pub(crate) use tile::TileHandle;

/// The six neighbour steps in axial coords, by direction (N, NE, SE, S, SW, NW).
const DIRECTION_STEP: [(i64, i64); 6] = [(0, 1), (1, 0), (1, -1), (0, -1), (-1, 0), (-1, 1)];

/// One battle tile.
#[derive(Default)]
pub(crate) struct TacticalTileData {
	pub col: i64,
	pub row: i64,
	pub q: i64,
	pub r: i64,
	pub level: i64,
	pub type_: i64,
	pub subtype: i64,
	pub blend_priority: i64,
	pub is_bad_terrain: bool,
	pub is_special_terrain: bool,
	pub is_defensible_terrain: bool,
	pub is_hiding_entity: bool,
	pub is_corpse_spawned: bool,
	pub is_discovered: bool,
	pub is_containing_items: bool,
	pub is_containing_items_flipped: bool,
	pub tv_height: i64,
	pub tv_terrain: i64,
	pub tv_level_disadvantage: i64,
	pub tv_total: i64,
	pub brush: String,
	/// The details spawned on it, in spawn order (see [`detail`]).
	pub details: Vec<detail::Detail>,
	/// `tile.setColor` (ARGB): the tile's tint, white unless a script sets it.
	pub color: u32,
	/// Who stands on it (a unit or an object), and whether it is an actor.
	pub occupant: Option<(SqStrongRef, bool)>,
	/// `tile.Properties` and `tile.Items`, made on first use.
	pub properties: Option<SqStrongRef>,
	pub items: Option<SqStrongRef>,
	/// Zones of control and of occupation exerted on it (see [`zone`]).
	pub zoc: zone::Zone,
	pub zoo: zone::Zone,
	/// The factions that see it, the player's fog flag (seen before, not now), and the
	/// stamp of the last vision update that saw it (see [`visibility`]).
	pub vis_mask: faction_set::FactionSet,
	pub fogged: bool,
	pub vis_stamp: u32,
	/// AI path heat (`Tactical.fillHeat`/`clearHeat`), weighted by the navigator's
	/// `HeatCost`.
	pub heat: i32,
	/// The highlighter's mark on it (`Tactical.getHighlighter()`).
	pub highlight: crate::native::tactical_highlighter::Highlight,
}

impl TacticalTileData {
	fn new(col: i64, row: i64) -> Self {
		TacticalTileData {
			col,
			row,
			q: col,
			r: row - col / 2,
			color: 0xffff_ffff,
			..Default::default()
		}
	}
}

/// Who stands on each tile, as sight needs it (from the entity host).
pub(crate) type OccupantLookup = Rc<dyn Fn() -> Vec<Option<visibility::Occupant>>>;

/// The tile a unit stands on, by its script identity (the entity host's records).
pub(crate) type UnitTileLookup = Rc<dyn Fn(i64) -> Option<usize>>;

/// What `tile.spawnObject(path)` runs: create the object entity `path` on tile `id`.
pub(crate) type ObjectSpawner = Rc<dyn Fn(&SQVM, &str, usize) -> Result<SqStrongRef, String>>;

/// What `tile.removeObject()` runs: take whatever stands on tile `id` off the map and
/// out of the battle (see `tactical_entities::placement::remove_object`).
pub(crate) type ObjectRemover = Rc<dyn Fn(&SQVM, usize) -> Result<(), String>>;

/// What else `tile.clear(flags)` clears on tile `id`: the battle effects lying there
/// (`None`: every flag).
pub(crate) type ClearHook = Rc<dyn Fn(&SQVM, usize, Option<i64>)>;

pub(crate) struct TacticalGridState {
	pub width: i64,
	pub height: i64,
	pub tiles: Vec<TacticalTileData>,
	/// The current vision update's stamp (`IsVisibleForEntity` compares against it).
	pub vis_stamp: u32,
	class: Option<SqClass<TileHandle>>,
	detail_class: Option<SqClass<detail::DetailHandle>>,
	/// The engine's own random stream: new details' mirroring, rotation and scale.
	engine_rng: EngineRng,
	/// Shared script code for `tile.Properties` objects (their `set`/`get`/… methods).
	properties_delegate: Option<SqStrongRef>,
	object_spawner: Option<ObjectSpawner>,
	object_remover: Option<ObjectRemover>,
	occupant_lookup: Option<OccupantLookup>,
	unit_tile_lookup: Option<UnitTileLookup>,
	clear_hook: Option<ClearHook>,
	/// `Tactical.getCamera()`'s fields.
	camera: camera::CameraState,
	camera_class: Option<SqClass<camera::CameraHandle>>,
	/// Game files (the picking masks come from the game data).
	file_reader: Option<FileReader>,
	/// The picking masks, read on first use (`None` inside if the data lacks them).
	hex_masks: Option<Option<Rc<picking::HexMasks>>>,
	/// Where `Tactical.addResource` hints resources.
	sprites: Sprites,
	/// The tiles with details that may change on their own (see `moving_sprites`).
	moving: Moving<usize>,
}

impl TacticalGridState {
	fn empty() -> Self {
		TacticalGridState {
			width: 0,
			height: 0,
			tiles: Vec::new(),
			vis_stamp: 0,
			class: None,
			detail_class: None,
			engine_rng: EngineRng::unseeded(),
			properties_delegate: None,
			object_spawner: None,
			object_remover: None,
			occupant_lookup: None,
			unit_tile_lookup: None,
			clear_hook: None,
			camera: camera::CameraState::default(),
			camera_class: None,
			file_reader: None,
			sprites: Sprites::default(),
			hex_masks: None,
			moving: Moving::default(),
		}
	}

	/// The picking masks, read from the game data the first time.
	fn hex_masks(&mut self) -> Option<Rc<picking::HexMasks>> {
		if self.hex_masks.is_none() {
			let read = self.file_reader.clone();
			self.hex_masks = Some(read.and_then(|r| picking::HexMasks::load(&*r)).map(Rc::new));
		}
		self.hex_masks.clone().flatten()
	}

	/// The flat index for square coords, or `None` if off the map.
	pub fn idx(&self, col: i64, row: i64) -> Option<usize> {
		(col >= 0 && col < self.width && row >= 0 && row < self.height).then(|| (row * self.width + col) as usize)
	}

	/// The flat index for axial coords, or `None` if off the map.
	pub fn idx_axial(&self, q: i64, r: i64) -> Option<usize> {
		self.idx(q, r + q / 2)
	}

	/// The neighbour of tile `id` in direction `dir` (0..6), if on the map.
	pub fn neighbour(&self, id: usize, dir: i64) -> Option<usize> {
		let (dq, dr) = *DIRECTION_STEP.get(usize::try_from(dir).ok()?)?;
		let t = &self.tiles[id];
		self.idx_axial(t.q + dq, t.r + dr)
	}

	/// The tile at axial coords as the original's navigator bounds-checks them
	/// (`0 <= q < width`, `-(q/2) <= r < height - q/2`), or `None`.
	pub fn idx_checked(&self, q: i64, r: i64) -> Option<usize> {
		(q >= 0 && q < self.width && r >= -(q / 2) && r < self.height - q / 2).then(|| ((r + q / 2) * self.width + q) as usize)
	}

	/// `Tactical.calculateTacticalValuesForTerrain()`: each tile's tactical values (as
	/// the original computes them; read by the AI and the navigator).
	///
	/// - `TVHeight`: twice its level, plus its height over each neighbour;
	/// - `TVTerrain`: -6 on bad terrain, +2 hiding, +2 defensible, then per neighbour +2
	///   if an object stands there, else +1 if that neighbour is bad terrain;
	/// - `TVLevelDisadvantage`: empty neighbours one level higher;
	/// - `TVTotal = TVHeight + TVTerrain`.
	pub fn calculate_tactical_values(&mut self) {
		let kinds: Vec<Option<bool>> = self.tiles.iter().map(|t| t.occupant.as_ref().map(|o| o.1)).collect();
		self.calculate_tactical_values_with(|i| kinds[i]);
	}

	/// [`calculate_tactical_values`](Self::calculate_tactical_values) with each tile's
	/// occupant given as `Some(is_actor)`.
	pub(crate) fn calculate_tactical_values_with(&mut self, occupant: impl Fn(usize) -> Option<bool>) {
		for id in 0..self.tiles.len() {
			let t = &self.tiles[id];
			let level = t.level;
			let mut height = level * 2;
			let mut terrain = if t.is_bad_terrain { -6 } else { 0 };
			if t.is_hiding_entity {
				terrain += 2;
			}
			if t.is_defensible_terrain {
				terrain += 2;
			}
			let mut disadvantage = 0;
			for n in (0..6).filter_map(|d| self.neighbour(id, d)) {
				let nt = &self.tiles[n];
				let d = level - nt.level;
				height += d;
				match occupant(n) {
					None if d == -1 => disadvantage += 1,
					Some(false) => {
						terrain += 2;
						continue;
					}
					_ => {}
				}
				if nt.is_bad_terrain {
					terrain += 1;
				}
			}
			let t = &mut self.tiles[id];
			t.tv_height = height;
			t.tv_terrain = terrain;
			t.tv_level_disadvantage = disadvantage;
			t.tv_total = height + terrain;
		}
	}

	/// The neighbours of tile `id` within one height level (where zones reach).
	fn zone_neighbours(&self, id: usize) -> Vec<usize> {
		let level = self.tiles[id].level;
		(0..6)
			.filter_map(|d| self.neighbour(id, d))
			.filter(|&n| (self.tiles[n].level - level).abs() <= 1)
			.collect()
	}

	/// `tile.addZoneOfControl(f)` / `addZoneOfOccupation(f)` (and the removes): apply to
	/// the neighbours within one level; faction 0 is ignored.
	pub fn apply_zone(&mut self, id: usize, f: i64, occupation: bool, add: bool) {
		if f == 0 {
			return;
		}
		for n in self.zone_neighbours(id) {
			let t = &mut self.tiles[n];
			let z = if occupation { &mut t.zoo } else { &mut t.zoc };
			if add {
				z.add(f)
			} else {
				z.remove(f)
			}
		}
	}

	/// A blank `w`×`h` map with no script classes (for tests).
	#[cfg(test)]
	pub(crate) fn for_test(w: i64, h: i64) -> Self {
		let mut g = TacticalGridState::empty();
		g.reset(w, h);
		g
	}

	fn reset(&mut self, w: i64, h: i64) {
		self.width = w.max(0);
		self.height = h.max(0);
		self.tiles = (0..self.width * self.height)
			.map(|i| TacticalTileData::new(i % self.width, i / self.width))
			.collect();
		self.moving.clear();
		if self.width > 0 && self.height > 0 {
			self.camera.bound_to_map(self.width, self.height);
		}
	}
}

pub(crate) type Grid = Rc<RefCell<TacticalGridState>>;

/// The tile centre in world pixels for square coords, on level 0.
pub(crate) fn tile_pos(col: i64, row: i64) -> (f64, f64) {
	let odd = if col % 2 != 0 { 40.0 } else { 0.0 };
	(80.0 + 120.0 * col as f64, 40.0 + 80.0 * row as f64 + odd)
}

/// World pixels per tile level: a raised tile sits this much further down the map
/// (probed: `Pos`, `tileToWorld` and actor positions all include it).
const LEVEL_HEIGHT: f64 = 40.0;

impl TacticalTileData {
	/// The tile's world position, as the original's tile-to-world helper: its centre,
	/// moved by its level. `tile.Pos`, `getDirection8To` and unit positions use it.
	pub(crate) fn world_pos(&self) -> (f64, f64) {
		let (x, y) = tile_pos(self.col, self.row);
		(x, y + LEVEL_HEIGHT * self.level as f64)
	}
}

/// The direction (0..6) from tile `a` towards tile `b`, as `tile.getDirectionTo` (the
/// original's, disassembled): not an angle but the signs of the axial deltas. Straight
/// up or down the column is 0 (down the rows) or 3 (otherwise, the tile itself
/// included); to the right 1, or 2 when `dr < 0`; to the left 4, or 5 when `dr > 0`.
pub(crate) fn direction_between(g: &TacticalGridState, a: usize, b: usize) -> usize {
	let (ta, tb) = (&g.tiles[a], &g.tiles[b]);
	axial_direction(tb.q - ta.q, tb.r - ta.r)
}

fn axial_direction(dq: i64, dr: i64) -> usize {
	match (dq.signum(), dr.signum()) {
		(0, 1) => 0,
		(0, _) => 3,
		(1, -1) => 2,
		(1, _) => 1,
		(-1, 1) => 5,
		_ => 4,
	}
}

#[cfg(test)]
mod direction_tests {
	use super::axial_direction;

	#[test]
	fn the_six_neighbours_and_far_tiles() {
		// The neighbour steps map to their own directions.
		for (d, (dq, dr)) in super::DIRECTION_STEP.iter().enumerate() {
			assert_eq!(axial_direction(*dq, *dr), d);
		}
		// Far tiles go by the signs alone.
		assert_eq!(axial_direction(5, 7), 1);
		assert_eq!(axial_direction(-5, -7), 4);
		assert_eq!(axial_direction(0, 0), 3);
	}

	/// `getDirectionTo`, `getDirection8To` and `getDistanceTo` from 7 tiles to every
	/// tile of a 32x32 battle map with raised tiles, and every tile's `Level` and
	/// `Pos`, as the original answers them (`testdata/directions.txt`, probed
	/// 2026-10-13).
	#[test]
	fn directions_match_the_original() {
		use super::{TacticalGridState, direction_between, hex_distance};
		use crate::native::world_query::direction8;
		let data = include_str!("testdata/directions.txt");
		let mut g = TacticalGridState::for_test(32, 32);
		// `L x level:posX:posY,...` per column, rows in order.
		let mut raised = 0;
		for line in data.lines().filter(|l| l.starts_with("L ")) {
			let (x, cells) = line[2..].split_once(' ').unwrap();
			for (y, cell) in cells.split(',').filter(|c| !c.is_empty()).enumerate() {
				let f: Vec<f64> = cell.split(':').map(|v| v.parse().unwrap()).collect();
				let id = g.idx(x.parse().unwrap(), y as i64).unwrap();
				g.tiles[id].level = f[0] as i64;
				raised += usize::from(f[0] > 0.0);
				assert_eq!(g.tiles[id].world_pos(), (f[1], f[2]), "Pos of {x},{y}");
			}
		}
		assert!(raised > 0);
		let mut rows = 0;
		for line in data.lines().filter(|l| l.starts_with("D ")) {
			let f: Vec<&str> = line.split(' ').collect();
			let (sx, sy) = f[1].split_once(',').unwrap();
			let src = g.idx(sx.parse().unwrap(), sy.parse().unwrap()).unwrap();
			let d6: Vec<u8> = f[2].bytes().map(|b| b - b'0').collect();
			let d8: Vec<u8> = f[3].bytes().map(|b| b - b'0').collect();
			let dist: Vec<i64> = f[4].split(',').filter(|x| !x.is_empty()).map(|x| x.parse().unwrap()).collect();
			let mut k = 0;
			for x in 0..32 {
				for y in 0..32 {
					let dst = g.idx(x, y).unwrap();
					let (a, b) = (&g.tiles[src], &g.tiles[dst]);
					assert_eq!(direction_between(&g, src, dst), d6[k] as usize, "dir6 {sx},{sy} -> {x},{y}");
					assert_eq!(direction8(a.world_pos(), b.world_pos()), d8[k] as i64, "dir8 {sx},{sy} -> {x},{y}");
					assert_eq!(hex_distance(a, b), dist[k], "dist {sx},{sy} -> {x},{y}");
					k += 1;
				}
			}
			rows += 1;
		}
		assert_eq!(rows, 7);
	}
}

/// Hex distance between two tiles.
pub(crate) fn hex_distance(a: &TacticalTileData, b: &TacticalTileData) -> i64 {
	hex_distance_to(a, b.q, b.r)
}

/// Hex distance from a tile to axial coords `(q, r)` (`getDistanceTo(x, y)`, `0x4daeb0`).
pub(crate) fn hex_distance_to(a: &TacticalTileData, q: i64, r: i64) -> i64 {
	let (dq, dr) = (a.q - q, a.r - r);
	(dq.abs() + dr.abs() + (dq + dr).abs()) / 2
}

/// Mint a script handle for tile `id`.
pub(crate) fn tile_handle(grid: &Grid, vm: &SQVM, id: usize) -> Option<SqStrongRef> {
	let class = grid.borrow().class.clone()?;
	Some(class.instantiate(vm, TileHandle { grid: grid.clone(), id }))
}

/// The tile id behind a script tile handle, or `None` if `value` isn't one.
pub(crate) fn tile_id_of(grid: &Grid, vm: &SQVM, value: &SqStrongRef) -> Option<usize> {
	let class = grid.borrow().class.clone()?;
	class.with_payload(vm, value, |t| t.id)
}

/// The tactical map service (cheap to clone; clones share it).
#[derive(Clone)]
pub struct TacticalGrid {
	grid: Grid,
}

impl Default for TacticalGrid {
	fn default() -> Self {
		Self::new()
	}
}

impl TacticalGrid {
	pub fn new() -> Self {
		TacticalGrid {
			grid: Rc::new(RefCell::new(TacticalGridState::empty())),
		}
	}

	/// The shared map state (for the entity host's placement).
	pub(crate) fn handle(&self) -> Grid {
		self.grid.clone()
	}

	/// Set what `tile.spawnObject` runs (the entity host's object creation).
	pub(crate) fn set_object_spawner(&self, spawner: ObjectSpawner) {
		self.grid.borrow_mut().object_spawner = Some(spawner);
	}

	/// Set what `tile.removeObject` runs (the entity host's removal).
	pub(crate) fn set_object_remover(&self, remover: ObjectRemover) {
		self.grid.borrow_mut().object_remover = Some(remover);
	}

	/// Set how sight learns who stands where (the entity host's records).
	pub(crate) fn set_occupant_lookup(&self, lookup: OccupantLookup) {
		self.grid.borrow_mut().occupant_lookup = Some(lookup);
	}

	/// Set how the camera's orientation overlays find their units' tiles.
	pub(crate) fn set_unit_tile_lookup(&self, lookup: UnitTileLookup) {
		self.grid.borrow_mut().unit_tile_lookup = Some(lookup);
	}

	/// A frame begins: no orientation overlay drawn yet (the camera's `render()` draws
	/// them, from the battle state's `onRender`).
	pub(crate) fn begin_frame(&self) {
		self.grid.borrow_mut().camera.overlays.begin_frame();
	}

	/// Set what else `tile.clear(flags)` clears (the battle effects on the tile).
	pub(crate) fn set_clear_hook(&self, hook: ClearHook) {
		self.grid.borrow_mut().clear_hook = Some(hook);
	}

	/// Let the map read game files (the picking masks).
	pub fn set_file_reader(&self, reader: FileReader) {
		let mut g = self.grid.borrow_mut();
		g.file_reader = Some(reader);
		g.hex_masks = None;
	}

	/// The tiles with items on the ground (`IsContainingItems`).
	/// Whether a battle map is set up.
	pub fn has_map(&self) -> bool {
		!self.grid.borrow().tiles.is_empty()
	}

	pub(crate) fn tiles_with_items(&self) -> Vec<usize> {
		let g = self.grid.borrow();
		(0..g.tiles.len()).filter(|&i| g.tiles[i].is_containing_items).collect()
	}

	/// The battle map as a renderer reads it (`transitions` aside), at real time `now`,
	/// `in_steps` the details turned as at the scene clock's last step (see
	/// `scene::steps`): their fades noted in `steps`.
	pub(crate) fn scene(&self, now: f32, in_steps: bool, steps: &crate::scene::steps::SceneSteps) -> crate::scene::tactical::TacticalScene {
		let g = self.grid.borrow();
		let tiles = g
			.tiles
			.iter()
			.map(|t| crate::scene::tactical::SceneTacticalTile {
				col: t.col,
				row: t.row,
				pos: tile_pos(t.col, t.row),
				level: t.level,
				terrain: t.type_,
				subtype: t.subtype,
				blend_priority: t.blend_priority,
				brush: t.brush.clone(),
				color: t.color,
				visible: t.vis_mask.contains(visibility::PLAYER),
				discovered: t.is_discovered,
				// Filled in with the units and the path (see `tactical_unveil`).
				unveils: false,
				highlight: t.highlight.look().map(|(brush, color)| (brush.to_string(), color)),
				details: t
					.details
					.iter()
					.filter(|d| d.borrow().visible)
					.map(|d| {
						let d = d.borrow();
						crate::scene::tactical::SceneTacticalDetail {
							brush: d.brush.clone(),
							flags: d.flags,
							mirrored: d.mirrored,
							offset: d.offset,
							rotation: if in_steps { d.drawn_rotation() } else { d.rotation },
							scale: d.scale,
							saturation: d.saturation,
							color: {
								if d.color.is_fading(now) {
									steps.need_finest();
								}
								d.color.shown(now)
							},
							frame: d.anim.frame(),
						}
					})
					.collect(),
			})
			.collect();
		let c = &g.camera;
		crate::scene::tactical::TacticalScene {
			width: g.width,
			height: g.height,
			tiles,
			transitions: Default::default(),
			camera: crate::scene::tactical::SceneCamera {
				pos: c.pos(),
				zoom: c.zoom(),
				level: c.level,
				flipped: c.flipped,
			},
			entities: Vec::new(),
			light: Default::default(),
			clouds: Vec::new(),
			hide_trees: false,
			particles: Vec::new(),
			effects: Vec::new(),
			pools: Vec::new(),
			overlay_icons: Vec::new(),
			path: Vec::new(),
			blocked_tiles: Vec::new(),
			rain: Vec::new(),
			rain_splats: Vec::new(),
			orientation: c.overlays.drawn().to_vec(),
		}
	}

	/// Tile `id`'s world position, its level included.
	pub(crate) fn tile_world_pos(&self, id: usize) -> Option<(f32, f32)> {
		self.grid.borrow().tiles.get(id).map(|t| {
			let (x, y) = t.world_pos();
			(x as f32, y as f32)
		})
	}

	/// Where the battle camera's centre is, read when called.
	/// What the battle camera shows on a `screen` of pixels, in world units, inside the
	/// map's bounds (the tiles at axial `(-2, -2)` and `(width + 1, height - width / 2 +
	/// 1)`, as the camera keeps to): `[left, right, bottom, top]`.
	pub(crate) fn view(&self, screen: (i64, i64)) -> [f32; 4] {
		let g = self.grid.borrow();
		let (cx, cy) = g.camera.pos();
		let zoom = g.camera.zoom();
		let (half_w, half_h) = (screen.0 as f64 * zoom / 2.0, screen.1 as f64 * zoom / 2.0);
		let at = |q: i64, r: i64| tile_pos(q, r + q / 2);
		let (min, max) = (at(-2, -2), at(g.width + 1, g.height - g.width / 2 + 1));
		[
			(cx - half_w).max(min.0) as f32,
			(cx + half_w).min(max.0) as f32,
			(cy - half_h).max(min.1) as f32,
			(cy + half_h).min(max.1) as f32,
		]
	}

	/// The tile drawn at world point `(x, y)` (`Tactical.worldToTile`), if it is on the
	/// map.
	pub(crate) fn tile_at(&self, x: f64, y: f64) -> Option<usize> {
		let mut g = self.grid.borrow_mut();
		let masks = g.hex_masks()?;
		let (q, r) = picking::world_to_tile(&g, &masks, g.camera.level, x, y);
		g.idx_axial(q, r)
	}

	pub(crate) fn camera_reader(&self) -> impl Fn() -> ((f32, f32), f32) + 'static {
		let grid = self.grid.clone();
		move || {
			let g = grid.borrow();
			let (x, y) = g.camera.pos();
			((x as f32, y as f32), g.camera.zoom() as f32)
		}
	}

	/// How the battle camera moves (see [`camera::BattleCamera`]).
	pub fn set_battle_camera(&self, mode: camera::BattleCamera) {
		self.grid.borrow_mut().camera.set_mode(mode);
	}

	/// A gliding battle camera's frame: `dt` seconds of real time at virtual time
	/// `now`, on a `screen`-sized screen, zooming in no closer than `zoom_min`.
	pub(crate) fn move_camera(&self, screen: (i32, i32), zoom_min: f32, dt: f32, now: f32) {
		if let Some(c) = self.grid.borrow_mut().camera.glide.as_mut() {
			c.set_screen(screen);
			c.set_zoom_min(zoom_min);
			c.update(dt, now);
		}
	}

	/// Draw new details' random looks from `rng`. Call before [`install`](Self::install).
	pub fn set_engine_rng(&self, rng: EngineRng) {
		self.grid.borrow_mut().engine_rng = rng;
	}

	/// Hint `Tactical.addResource` paths to `sprites`. Call before [`install`](Self::install).
	pub fn set_sprites(&self, sprites: Sprites) {
		self.grid.borrow_mut().sprites = sprites;
	}

	/// Install the tile classes, the camera and the map natives on the `Tactical` table.
	pub fn install(&self, vm: &mut SQVM) {
		let class = tile::build_class(vm);
		let (sprites, moving) = {
			let g = self.grid.borrow();
			(g.sprites.clone(), g.moving.clone())
		};
		let detail_class = detail::build_class(vm, &sprites, &moving);
		let properties_delegate = tile::properties_delegate(vm, &self.grid);
		let camera_class = camera::build_class(vm, &self.grid);
		let camera = camera::instance(vm, &camera_class, &self.grid);
		{
			let mut g = self.grid.borrow_mut();
			g.class = Some(class);
			g.detail_class = Some(detail_class);
			g.properties_delegate = Some(properties_delegate);
			g.camera_class = Some(camera_class);
		}
		let get_camera = vm.make_closure(move || camera.clone());
		// Tactical.worldToTile(pos): the tile drawn at a world point (see `picking`).
		let world_to_tile = {
			let g = self.grid.clone();
			vm.make_closure(move |vm: &SQVM, pos: Retained| -> Result<SqStrongRef, String> {
				let x = pos.0.get_member::<Number>(vm, "X").ok_or("worldToTile: expected a vector")?.0;
				let y = pos.0.get_member::<Number>(vm, "Y").ok_or("worldToTile: expected a vector")?.0;
				let mut g = g.borrow_mut();
				let masks = g.hex_masks().ok_or("worldToTile: the game data has no gfx/masks/hex_mask.0-3")?;
				let (q, r) = picking::world_to_tile(&g, &masks, g.camera.level, x, y);
				let mut t = SqTable::new(vm);
				t.set("X", q);
				t.set("Y", r);
				Ok(t.to_strong())
			})
		};

		// Tactical.screenToTile(x, y): the tile under a screen pixel (clicks).
		let screen_to_tile = {
			let g = self.grid.clone();
			vm.make_closure(move |vm: &SQVM, x: Number, y: Number| -> Result<SqStrongRef, String> {
				let mut g = g.borrow_mut();
				let (wx, wy) = camera::screen_to_world(&g.camera, vm, x.0, y.0);
				let masks = g.hex_masks().ok_or("screenToTile: the game data has no gfx/masks/hex_mask.0-3")?;
				let (q, r) = picking::world_to_tile(&g, &masks, g.camera.level, wx, wy);
				let mut t = SqTable::new(vm);
				t.set("X", q);
				t.set("Y", r);
				Ok(t.to_strong())
			})
		};

		let resize = {
			let g = self.grid.clone();
			vm.make_closure(move |w: i64, h: i64| g.borrow_mut().reset(w, h))
		};
		let clear_scene = {
			let g = self.grid.clone();
			vm.make_closure(move || g.borrow_mut().reset(0, 0))
		};
		let get_map_size = {
			let g = self.grid.clone();
			vm.make_closure(move |vm: &SQVM| -> SqStrongRef {
				let (w, h) = {
					let g = g.borrow();
					(g.width, g.height)
				};
				let mut t = SqTable::new(vm);
				t.set("X", w);
				t.set("Y", h);
				t.to_strong()
			})
		};
		let get_tile_square = {
			let g = self.grid.clone();
			vm.make_closure(move |vm: &SQVM, x: i64, y: i64| -> Result<SqStrongRef, String> {
				let id = off_map::square_or_nearest(&g, vm, x, y)?;
				tile_handle(&g, vm, id).ok_or_else(|| "getTileSquare: no tile class".to_string())
			})
		};
		let is_valid_square = {
			let g = self.grid.clone();
			vm.make_closure(move |x: i64, y: i64| g.borrow().idx(x, y).is_some())
		};
		let is_valid_tile = {
			let g = self.grid.clone();
			vm.make_closure(move |q: i64, r: i64| g.borrow().idx_axial(q, r).is_some())
		};
		let get_tile_1 = {
			let g = self.grid.clone();
			vm.make_closure(move |vm: &SQVM, coords: Retained| -> Result<SqStrongRef, String> {
				let q = coords.0.get_member::<i64>(vm, "X").ok_or("getTile: coords need X")?;
				let r = coords.0.get_member::<i64>(vm, "Y").ok_or("getTile: coords need Y")?;
				let id = g.borrow().idx_axial(q, r).ok_or_else(|| format!("getTile: ({q}, {r}) is off the map"))?;
				tile_handle(&g, vm, id).ok_or_else(|| "getTile: no tile class".to_string())
			})
		};
		let get_tile_2 = {
			let g = self.grid.clone();
			vm.make_closure(move |vm: &SQVM, q: i64, r: i64| -> Result<SqStrongRef, String> {
				let id = g.borrow().idx_axial(q, r).ok_or_else(|| format!("getTile: ({q}, {r}) is off the map"))?;
				tile_handle(&g, vm, id).ok_or_else(|| "getTile: no tile class".to_string())
			})
		};
		let get_tile_dispatch = make_dispatcher(vm, "getTile");
		let tile_to_world_1 = {
			let grid = self.grid.clone();
			vm.make_closure(move |vm: &SQVM, coords: Retained| -> Result<Option<SqStrongRef>, String> {
				let q = coords.0.get_member::<i64>(vm, "X").ok_or("tileToWorld: coords need X")?;
				let r = coords.0.get_member::<i64>(vm, "Y").ok_or("tileToWorld: coords need Y")?;
				let g = grid.borrow();
				vec(vm, g.idx_axial(q, r).map_or_else(|| tile_pos(q, r + q / 2), |id| g.tiles[id].world_pos()))
			})
		};
		let tile_to_world_2 = vm.make_closure(|vm: &SQVM, q: i64, r: i64| vec(vm, tile_pos(q, r + q / 2)));
		let tile_to_world_dispatch = make_dispatcher(vm, "tileToWorld");
		let queries = query::natives(vm, &self.grid);
		let heat_natives = heat::natives(vm, &self.grid);
		let query_tiles_dispatch = make_dispatcher(vm, "queryTilesInRange");
		let calculate_tv = {
			let g = self.grid.clone();
			vm.make_closure(move || g.borrow_mut().calculate_tactical_values())
		};
		// Rendering-only map natives.
		// Tactical.addResource(path): a script's hint that an atlas or sound will be
		// needed in the coming battle. Loading starts on the host's side; the original's
		// Tactical.loadResources() (which then loads them all) stays a no-op.
		let add_resource = {
			let sprites = self.grid.borrow().sprites.clone();
			vm.make_closure(move |vm: &SQVM, path: Retained| {
				if let Some(path) = path.0.get_value::<String>(vm) {
					sprites.hint(&path);
				}
			})
		};

		// Tactical.loadResources(): the original's flush of the hints above. Each hint
		// has already gone to the on-demand loader, so there is nothing left to do and no
		// script can tell; a plain no-op, not a stub (it is not a gap).
		let load_resources = vm.make_closure(|| {});
		// Still stubs (see `stubs`).
		let stubs: Vec<(&str, SqStrongRef)> = ["setAmbientColor"]
			.into_iter()
			.map(|name| (name, crate::native::stubs::closure(vm, crate::native::stubs::name(format!("Tactical.{name}")))))
			.collect();
		let mut tactical = SqTable::root(vm)
			.get::<SqTable>("Tactical")
			.expect("Tactical table must be installed before TacticalGrid");
		tactical.set("loadResources", load_resources);
		tactical.set("resizeScene", resize);
		tactical.set("clearScene", clear_scene);
		tactical.set("getMapSize", get_map_size);
		tactical.set("getTileSquare", get_tile_square);
		tactical.set("isValidTileSquare", is_valid_square);
		tactical.set("isValidTile", is_valid_tile);
		tactical.set(&overload_key("getTile", 1), get_tile_1);
		tactical.set(&overload_key("getTile", 2), get_tile_2);
		tactical.set("getTile", get_tile_dispatch);
		tactical.set(&overload_key("tileToWorld", 1), tile_to_world_1);
		tactical.set(&overload_key("tileToWorld", 2), tile_to_world_2);
		tactical.set("tileToWorld", tile_to_world_dispatch);
		tactical.set("worldToTile", world_to_tile);
		tactical.set("screenToTile", screen_to_tile);
		tactical.set("getCamera", get_camera);
		tactical.set("calculateTacticalValuesForTerrain", calculate_tv);
		for (name, f) in queries {
			tactical.set(&name, f);
		}
		tactical.set("queryTilesInRange", query_tiles_dispatch);
		for (name, f) in heat_natives {
			tactical.set(name, f);
		}
		tactical.set("addResource", add_resource);
		for (name, stub) in stubs {
			tactical.set(name, stub);
		}
	}

	/// The battle camera's zoom: world units a pixel of the game's screen.
	pub(crate) fn camera_zoom(&self) -> f64 {
		self.grid.borrow().camera.zoom()
	}

	/// Mirror the battle camera's view, or not (its `IsFlipped`, which scripts only read:
	/// in the original the engine flips it).
	pub(crate) fn set_camera_flipped(&self, flipped: bool) {
		self.grid.borrow_mut().camera.flipped = flipped;
	}

	/// Turn the details with a torque to virtual time `now`, in seconds (see `spin`),
	/// and step their animations to virtual millisecond `now_ms` (see `sprite_anim`).
	/// Those on tiles the player sees are drawn at real time `real`: a draw finishes a
	/// fade (see `SpriteColor`), which may hide the detail. Only the tiles noted as
	/// having details that may change are visited (see `moving_sprites`), and forgotten
	/// once none may.
	///
	/// At a step of the scene's clock (`stepped`, see `scene::steps`) the rotations drawn
	/// are taken again. Returns how fast the fastest turning detail shown moves, in world
	/// units a virtual second, when asked to `measure` (else 0).
	pub(crate) fn turn(&self, now: f32, now_ms: u32, real: f32, stepped: bool, measure: bool) -> f32 {
		let g = self.grid.borrow();
		let mut fastest = 0.0f32;
		g.moving.retain(|tile| {
			let Some(t) = g.tiles.get(tile) else { return false };
			let seen = t.vis_mask.contains(visibility::PLAYER);
			let mut moves = false;
			for d in &t.details {
				let mut d = d.borrow_mut();
				let d = &mut *d;
				if d.turns() {
					d.turn(now, stepped);
					if measure && d.visible && d.spin.torque != 0.0 {
						let looks = g.sprites.with(|r| r.get(&d.brush).map(|(_, s)| ((s.width as f32, s.height as f32), (f32::from(s.offset_x), f32::from(s.offset_y)))));
						if let Some((size, offset)) = looks {
							fastest = fastest.max(d.spin_speed(size, offset));
						}
					}
					if let Some(scale) = d.anim.step_on(&g.sprites, &d.brush, now_ms) {
						d.scale = scale;
					}
				}
				if seen && d.visible && d.color.draw(real).hide {
					d.visible = false;
				}
				moves |= d.may_move();
			}
			moves
		});
		fastest
	}

	/// The details that may change on their own though their tile isn't noted as moving
	/// (see `moving_sprites`): a pass would miss them. Tests.
	pub(crate) fn unnoted_moving(&self) -> usize {
		let g = self.grid.borrow();
		(0..g.tiles.len())
			.filter(|&i| !g.moving.has(i))
			.flat_map(|i| g.tiles[i].details.iter())
			.filter(|d| d.borrow().may_move())
			.count()
	}

	/// Release every script reference the map holds. Call before the VM is closed.
	pub fn clear(&self) {
		let mut g = self.grid.borrow_mut();
		g.tiles.clear();
		g.moving.clear();
		g.class = None;
		g.detail_class = None;
		g.properties_delegate = None;
		g.object_spawner = None;
		g.object_remover = None;
		g.occupant_lookup = None;
		g.unit_tile_lookup = None;
		g.clear_hook = None;
		g.camera_class = None;
		g.camera.callbacks = Default::default();
	}
}

/// `createVec(x, y)`.
fn vec(vm: &SQVM, (x, y): (f64, f64)) -> Result<Option<SqStrongRef>, String> {
	let create = SqTable::root(vm).get::<SqStrongRef>("createVec").ok_or("createVec missing")?;
	create.call_ret::<SqStrongRef, _>(vm, (x, y))
}
