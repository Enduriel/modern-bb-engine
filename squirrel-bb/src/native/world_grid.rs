//! The World hex tile grid: the `WorldTile` native class + the `World` grid natives,
//! backed by a host-owned [`WorldGrid`].
//!
//! Coordinates (verified from the scripts): the grid is stored in **square/offset**
//! coords `(col, row)` (the worldgen iteration order, `getTileSquare(col,row)`), and
//! each tile also carries **axial** coords `Coords = (q, r)` with `q = col`,
//! `r = row - col/2` (`getTile(q,r)`). Hex neighbours are uniform in axial space —
//! [`DIRECTION_STEP`] indexed by `Const.Direction` (N=0…NW=5). This provides the grid
//! structure and the native API that `worldmap_generator.fill` runs on.

use std::cell::RefCell;
use std::rc::Rc;

use squirrel::{
	ClassBuilder, Number, Native, Retained, SQVM, SqArray, SqBytes, SqClass, SqStrongRef, SqTable, StackArgs, This,
	Variadic,
};

use super::fog_mask::{FogMask, FogPatch, FogStamp};
use crate::scene::SceneRegionText;
use super::overload::{Overload, make_dispatcher, overload_key};
use super::world_detail::{self, Clear, Detail, DetailData, DetailHandle};
use crate::native::{EngineRng, EntityIds, Sprites};
use super::world_layout;
use super::world_nav::{self, NavSettings, NavTile};
use super::world_search;

mod original;
pub(crate) use original::brush_name;

/// Axial neighbour deltas `(dq, dr)` indexed by `Const.Direction`: N, NE, SE, S, SW, NW.
const DIRECTION_STEP: [(i64, i64); 6] = [(0, 1), (1, 0), (1, -1), (0, -1), (-1, 0), (-1, 1)];

/// One tile's data. Identity/coords are fixed at `resizeScene`; the gameplay fields
/// are what worldgen reads/writes and what `clearTiles` resets.
#[derive(Clone, Default)]
struct TileData {
	id: usize,
	col: i64,
	row: i64,
	q: i64,
	r: i64,
	type_: i64,
	tactical_type: i64,
	subregion: i64,
	region: i64,
	road_directions: i64,
	river_directions: i64,
	heat: i64,
	z_offset: f64,
	is_occupied: bool,
	is_flipped: bool,
	is_discovered: bool,
	has_river: bool,
	/// The terrain brush name (`setBrush`/`getBrush`/`resetBrush`). Worldgen reads it
	/// back (`tile.getBrush() != "world_highlands_03"`), so it is real state, not a
	/// render no-op. `""` when unset, like the original (probed 2026-10-13).
	brush: String,
	/// The decorations spawned on the tile, in spawn order.
	details: Vec<Detail>,
}

/// The map's share of a save: its size, every tile's gameplay state, the fog of
/// war's mask (the original saves its mask too) and the region names. Details
/// (decorations) are visual and not kept.
struct GridPart(Rc<RefCell<GridState>>);

impl super::save::SavePart for GridPart {
	fn save(&self, cx: &super::save::SaveCx) -> Result<(), String> {
		let g = self.0.borrow();
		let mut w = cx.writer.borrow_mut();
		w.i64(g.width);
		w.i64(g.height);
		for t in &g.tiles {
			for v in [t.type_, t.tactical_type, t.subregion, t.region, t.road_directions, t.river_directions, t.heat] {
				w.i64(v);
			}
			w.f64(t.z_offset);
			for b in [t.is_occupied, t.is_flipped, t.is_discovered, t.has_river] {
				w.bool(b);
			}
			w.str(&t.brush);
		}
		g.fog.save(&mut w);
		w.u32(g.region_texts.len() as u32);
		for t in &g.region_texts {
			w.i64(t.id);
			w.bytes(&t.text);
			for v in [t.scale, t.rotation, t.pos.0, t.pos.1] {
				w.f32(v);
			}
		}
		Ok(())
	}

	fn load(&self, cx: &super::save::LoadCx) -> Result<(), String> {
		let mut g = self.0.borrow_mut();
		let mut r = cx.reader.borrow_mut();
		let (w, h) = (r.i64()?, r.i64()?);
		g.width = w;
		g.height = h;
		let mut tiles = Vec::with_capacity((w.max(0) * h.max(0)) as usize);
		for i in 0..(w * h).max(0) {
			let mut t = TileData::new(i as usize, i % w, i / w);
			t.type_ = r.i64()?;
			t.tactical_type = r.i64()?;
			t.subregion = r.i64()?;
			t.region = r.i64()?;
			t.road_directions = r.i64()?;
			t.river_directions = r.i64()?;
			t.heat = r.i64()?;
			t.z_offset = r.f64()?;
			t.is_occupied = r.bool()?;
			t.is_flipped = r.bool()?;
			t.is_discovered = r.bool()?;
			t.has_river = r.bool()?;
			t.brush = r.str()?;
			tiles.push(t);
		}
		g.tiles = tiles;
		g.fog.load(&mut r)?;
		bound_camera(&g);
		let count = r.u32()?;
		g.region_texts = (0..count)
			.map(|_| {
				Ok(SceneRegionText {
					id: r.i64()?,
					text: r.bytes()?,
					scale: r.f32()?,
					rotation: r.f32()?,
					pos: (r.f32()?, r.f32()?),
				})
			})
			.collect::<Result<_, String>>()?;
		Ok(())
	}
}

/// A navigation path returned by `World.getNavigator().findPath(...)`: the waypoint tile
/// ids AFTER the start (callers track the start themselves), with a cursor the road
/// phases advance via `getCurrent()`/`pop()`. Empty if unreachable.
#[derive(Clone)]
struct PathData {
	grid: Rc<RefCell<GridState>>,
	tiles: Vec<usize>,
	cursor: usize,
}

impl PathData {
	/// The axial `{X, Y}` of waypoint `at`.
	fn coords_at(&self, vm: &SQVM, at: usize) -> Option<SqStrongRef> {
		let id = *self.tiles.get(at)?;
		let g = self.grid.borrow();
		Some(xy_table(vm, g.tiles[id].q, g.tiles[id].r))
	}

	/// The tile id of axial `{X, Y}` coords, if on the map.
	fn tile_of(&self, vm: &SQVM, coords: &SqStrongRef) -> Option<usize> {
		let q = coords.get_member::<i64>(vm, "X")?;
		let r = coords.get_member::<i64>(vm, "Y")?;
		self.grid.borrow().idx(q, r + q / 2)
	}
}

impl TileData {
	fn new(id: usize, col: i64, row: i64) -> Self {
		// Axial: q = col, r = row - floor(col/2) (col >= 0 over the whole grid, so
		// truncating division == floor here).
		TileData {
			id,
			col,
			row,
			q: col,
			r: row - col / 2,
			..TileData::default()
		}
	}
}

/// The grid backing store (host-owned, shared with the native closures).
struct GridState {
	width: i64,
	height: i64,
	tiles: Vec<TileData>,
	/// The `WorldTile` class, used to mint tile handles from inside tile methods
	/// (`getNextTile`) and the grid natives. Set during `install`.
	class: Option<SqClass<TileHandle>>,
	/// The `WorldDetail` class, used by `tile.spawnDetail` to mint detail handles.
	/// Set during `install`; released by `clear` (same teardown contract as `class`,
	/// so no `SqStrongRef` is captured inside a VM-owned closure).
	detail_class: Option<SqClass<DetailHandle>>,
	/// The navigation-path class, used by `World.getNavigator().findPath` to mint
	/// path handles. Set during `install`; released by `clear`.
	nav_path_class: Option<SqClass<PathData>>,
	/// Where brushes set on tiles and details hint their atlases.
	sprites: Sprites,
	/// The engine's random stream: new details' mirroring, scale and rotation.
	engine_rng: EngineRng,
	/// The entity ids every entity shares; details take theirs from it.
	ids: EntityIds,
	/// The details that may turn (see `world_detail::Spinning`).
	spinning: world_detail::Spinning,
	/// The camera the map keeps in view (see [`bound_camera`]).
	camera: Option<super::world_camera::WorldCamera>,
	/// Fog of war is on (`setFogOfWar`).
	fog_on: bool,
	/// Where the fog of war is drawn.
	fog: FogMask,
	/// Region names (`spawnRegionText`), in spawn order.
	region_texts: Vec<SceneRegionText>,
}

impl GridState {
	/// The flat index for square coords, or `None` if out of bounds.
	fn idx(&self, col: i64, row: i64) -> Option<usize> {
		(col >= 0 && col < self.width && row >= 0 && row < self.height)
			.then(|| (row * self.width + col) as usize)
	}
}

/// The payload behind a `WorldTile` instance: a borrowed handle into the grid (an
/// `Rc` to the store + the tile's flat id). Cheap to clone.
#[derive(Clone)]
struct TileHandle {
	grid: Rc<RefCell<GridState>>,
	id: usize,
}

impl TileHandle {
	fn with<R>(&self, f: impl FnOnce(&TileData) -> R) -> R {
		f(&self.grid.borrow().tiles[self.id])
	}
	fn with_mut<R>(&self, f: impl FnOnce(&mut TileData) -> R) -> R {
		f(&mut self.grid.borrow_mut().tiles[self.id])
	}
	/// The flat id of the neighbour in `dir` (0..6), or `None` (off-grid / bad dir).
	fn neighbour_id(&self, dir: i64) -> Option<usize> {
		let step = *DIRECTION_STEP.get(usize::try_from(dir).ok()?)?;
		let (q, r) = self.with(|d| (d.q, d.r));
		let (nq, nr) = (q + step.0, r + step.1);
		self.grid.borrow().idx(nq, nr + nq / 2) // axial -> square
	}
}

fn xy_table(vm: &SQVM, x: i64, y: i64) -> SqStrongRef {
	let mut t = SqTable::new(vm);
	t.set("X", x);
	t.set("Y", y);
	t.to_strong()
}

/// Hex distance between two tiles (axial cube distance).
fn hex_distance(a: &TileHandle, b: &TileHandle) -> i64 {
	let (bq, br) = b.with(|d| (d.q, d.r));
	hex_distance_to(a, bq, br)
}

/// Hex distance from a tile to axial coords `(q, r)` (`getDistanceTo(x, y)`, `0x4fa950`).
fn hex_distance_to(a: &TileHandle, q: i64, r: i64) -> i64 {
	let (aq, ar) = a.with(|d| (d.q, d.r));
	let (dq, dr) = (aq - q, ar - r);
	(dq.abs() + dr.abs() + (dq + dr).abs()) / 2
}

/// The host-owned World hex tile grid service. Install into a VM (after the `World`
/// table exists), then `World.resizeScene(w,h)` allocates it. Holds the `WorldTile`
/// class (an `SqStrongRef`), so [`clear`](Self::clear) it before the VM closes
/// ([`Engine`](crate::Engine) does this automatically).
#[derive(Clone)]
pub struct WorldGrid {
	grid: Rc<RefCell<GridState>>,
}

impl Default for WorldGrid {
	fn default() -> Self {
		Self::new()
	}
}

impl WorldGrid {
	pub fn new() -> Self {
		WorldGrid {
			grid: Rc::new(RefCell::new(GridState {
				width: 0,
				height: 0,
				tiles: Vec::new(),
				class: None,
				detail_class: None,
				nav_path_class: None,
				sprites: Sprites::default(),
				engine_rng: EngineRng::unseeded(),
				ids: EntityIds::default(),
				spinning: Rc::default(),
				camera: None,
				fog_on: true,
				fog: FogMask::default(),
				region_texts: Vec::new(),
			})),
		}
	}

	/// Draw new details' random looks from `rng`. Call before [`install`](Self::install).
	pub fn set_engine_rng(&self, rng: EngineRng) {
		self.grid.borrow_mut().engine_rng = rng;
	}

	/// The entity ids details take theirs from (shared with every entity).
	pub fn set_entity_ids(&self, ids: EntityIds) {
		self.grid.borrow_mut().ids = ids;
	}

	/// Keep `camera`'s view on the map. Call before [`install`](Self::install).
	pub(crate) fn set_camera(&self, camera: super::world_camera::WorldCamera) {
		self.grid.borrow_mut().camera = Some(camera);
	}

	/// Hint the atlases of brushes set on tiles and details to `sprites`. Call before
	/// [`install`](Self::install).
	pub fn set_sprites(&self, sprites: Sprites) {
		self.grid.borrow_mut().sprites = sprites;
	}

	/// Build the `WorldTile` class and add the grid natives to the existing `World`
	/// table. The `World` table must already be installed (it is — by the stateless
	/// native config).
	pub fn install(&self, vm: &mut SQVM) {
		// Build the detail class first so `spawnDetail` can mint instances (the tile
		// methods fetch it from the grid at call time, capturing no `SqStrongRef`).
		let (sprites, spinning) = {
			let g = self.grid.borrow();
			(g.sprites.clone(), g.spinning.clone())
		};
		let detail_class = world_detail::build_class(vm, &sprites, &spinning);
		self.grid.borrow_mut().detail_class = Some(detail_class);
		let nav_path_class = build_world_navigation_path_class(vm);
		self.grid.borrow_mut().nav_path_class = Some(nav_path_class);
		let class = build_world_tile_class(vm);
		self.grid.borrow_mut().class = Some(class);

		// Grid natives (each captures the shared grid). Built before fetching the
		// World table handle (closure creation needs `&mut vm`).
		let resize = {
			let g = self.grid.clone();
			vm.make_closure(move |w: i64, h: i64| {
				let mut g = g.borrow_mut();
				g.width = w;
				g.height = h;
				g.tiles = if w > 0 && h > 0 {
					(0..(w * h))
						.map(|i| TileData::new(i as usize, i % w, i / w))
						.collect()
				} else {
					Vec::new()
				};
				// The original clears its fog mask here too (`0x4efba9`).
				g.fog.reset(fog_extent(w, h));
				g.region_texts.clear();
				bound_camera(&g);
			})
		};
		let clear = {
			let g = self.grid.clone();
			vm.make_closure(move || {
				let mut g = g.borrow_mut();
				for t in &mut g.tiles {
					*t = TileData::new(t.id, t.col, t.row); // reset gameplay, keep coords
				}
			})
		};
		let get_map_size = {
			let g = self.grid.clone();
			vm.make_closure(move |vm: &SQVM| -> SqStrongRef {
				let (w, h) = {
					let g = g.borrow();
					(g.width, g.height)
				};
				xy_table(vm, w, h)
			})
		};
		let get_tile_square = {
			let g = self.grid.clone();
			vm.make_closure(
				move |vm: &SQVM, col: i64, row: i64| -> Option<SqStrongRef> {
					tile_at(&g, vm, col, row)
				},
			)
		};
		// World.getAllTilesOfRegion(region): every tile of that region, column by column
		// (disassembled, 0x4f3ce0).
		let get_all_tiles_of_region = {
			let g = self.grid.clone();
			vm.make_closure(move |vm: &SQVM, region: i64| -> SqStrongRef {
				let mut list = SqArray::new(vm);
				let (w, h) = {
					let g = g.borrow();
					(g.width, g.height)
				};
				for col in 0..w {
					for row in 0..h {
						let matches = {
							let g = g.borrow();
							g.idx(col, row).is_some_and(|i| g.tiles[i].region == region)
						};
						if matches {
							if let Some(tile) = tile_at(&g, vm, col, row) {
								list.push(tile);
							}
						}
					}
				}
				list.to_strong()
			})
		};
		// World.getTile is a Sqrat overload: getTile(coords) [1] and getTile(q, r) [2].
		// Worldgen calls getTile(path.getCurrent()) / getTile(worldToTile(..)) [1-arg],
		// and entity getTile delegates with (q, r) [2-arg].
		let get_tile_1 = {
			let g = self.grid.clone();
			vm.make_closure(move |vm: &SQVM, coords: Retained| -> Option<SqStrongRef> {
				// coords is an {X, Y} table or a vec2i; read axial q/r via member access.
				let q = coords.0.get_member::<i64>(vm, "X")?;
				let r = coords.0.get_member::<i64>(vm, "Y")?;
				tile_at(&g, vm, q, r + q / 2) // axial -> square
			})
		};
		let get_tile_2 = {
			let g = self.grid.clone();
			vm.make_closure(move |vm: &SQVM, q: i64, r: i64| -> Option<SqStrongRef> {
				tile_at(&g, vm, q, r + q / 2) // axial -> square
			})
		};
		let get_tile_dispatch = make_dispatcher(vm, "getTile");
		let is_valid_square = {
			let g = self.grid.clone();
			vm.make_closure(move |col: i64, row: i64| -> bool {
				g.borrow().idx(col, row).is_some()
			})
		};
		let is_valid_tile = {
			let g = self.grid.clone();
			vm.make_closure(move |q: i64, r: i64| -> bool {
				g.borrow().idx(q, r + q / 2).is_some()
			})
		};
		let num_of_type = {
			let g = self.grid.clone();
			vm.make_closure(move |types: SqArray| -> i64 {
				let wanted = types.to_vec::<i64>();
				g.borrow()
					.tiles
					.iter()
					.filter(|d| wanted.contains(&d.type_))
					.count() as i64
			})
		};
		// World.queryTilesInRange(center, minRange, maxRange, callback, userdata):
		// for every tile (not the centre) whose hex distance from `center` is in
		// [min, max], call `callback(tile, userdata)`, in the original's walk order. The world callers pass a `bindenv`'d closure that
		// mutates `userdata` (e.g. tallying terrain types in a region); the return value
		// is unused. (Tactical.queryTilesInRange is a separate, overloaded native with more
		// filters; see `tactical_grid::query`.)
		let query_tiles_in_range = {
			let g = self.grid.clone();
			vm.make_closure(
				move |vm: &SQVM,
				      center: Native<TileHandle>,
				      min: i64,
				      max: i64,
				      callback: SqStrongRef,
				      userdata: Retained|
				      -> Result<(), String> {
					// The original's range walk (`hex_range`): sector order, centre
					// excluded; then the inner rings closer than `min` are skipped. No grid
					// borrow is held across the callback (it runs arbitrary script that
					// may re-enter the grid).
					let (cq, cr) = center.0.with(|d| (d.q, d.r));
					let ids = {
						let gb = g.borrow();
						super::hex_range::tiles_in_range((cq, cr), max, gb.tiles.len(), |q, r| gb.idx(q, r + q / 2))
					};
					for id in ids {
						let (q, r) = {
							let gb = g.borrow();
							(gb.tiles[id].q, gb.tiles[id].r)
						};
						let dist = ((q - cq).abs() + (r - cr).abs() + (q - cq + r - cr).abs()) / 2;
						if dist < min {
							continue;
						}
						if let Some(tile) = tile_at(&g, vm, q, r + q / 2) {
							callback.call(vm, (tile, userdata.0.clone()))?;
						}
					}
					Ok(())
				},
			)
		};

		// World.updateTilesWithHeat(settlementTiles, radius): accumulate (radius - d)
		// onto each grid tile's HeatFromSettlements for every settlement tile within hex
		// distance d <= radius (the settlement-proximity field AI/pathfinding weight on).
		let update_tiles_with_heat = {
			let g = self.grid.clone();
			vm.make_closure(
				move |vm: &SQVM, tiles: SqArray, radius: i64| -> Result<(), String> {
					// Resolve the settlement tile coords first (reading tile.X/Y borrows
					// the grid), then mutate heat under a single borrow.
					let mut centers: Vec<(i64, i64)> = Vec::new();
					for i in 0..tiles.len() {
						if let Some(t) = tiles.get::<SqStrongRef>(i)
							&& let (Some(q), Some(r)) =
								(t.get_member::<i64>(vm, "X"), t.get_member::<i64>(vm, "Y"))
						{
							centers.push((q, r));
						}
					}
					let mut grid = g.borrow_mut();
					for (cq, cr) in centers {
						for dq in -radius..=radius {
							let lo = (-radius).max(-dq - radius);
							let hi = radius.min(-dq + radius);
							for dr in lo..=hi {
								let d = (dq.abs() + dr.abs() + (dq + dr).abs()) / 2;
								if d > radius {
									continue;
								}
								let (nq, nr) = (cq + dq, cr + dr);
								if let Some(id) = grid.idx(nq, nr + nq / 2) {
									grid.tiles[id].heat += radius - d;
								}
							}
						}
					}
					Ok(())
				},
			)
		};

		// World.findNextRegion(tile, dist, type): the region id of the nearest already-
		// assigned (Region != 0) tile within `dist` whose Type matches `type` (type 0 ==
		// any), else 0 — the flood-fill grouping that buildLabels uses to merge tiles
		// into regions.
		// World.uncoverFogOfWar(pos, radius): mark tiles discovered, exactly like the
		// original (disassembly, 2026-10-13; verified on 8 probed shapes): with
		// rt = radius / 100, every tile within hex distance trunc(rt) + 3 of the tile
		// under `pos`, clipped to columns [q - trunc(rt/2) - 3, q + trunc(rt/2) + 4) and
		// rows [row - trunc(rt) - 3, row + trunc(rt) + 3). Then it stamps the fog
		// mask at `pos` with scale rt (see `fog_mask`).
		let uncover_fog_of_war = {
			let g = self.grid.clone();
			vm.make_closure(move |vm: &SQVM, pos: Retained, radius: Number| {
				let px = pos.0.get_member::<Number>(vm, "X").map_or(0.0, |n| n.0);
				let py = pos.0.get_member::<Number>(vm, "Y").map_or(0.0, |n| n.0);
				let mut grid = g.borrow_mut();
				let (w, h) = (grid.width, grid.height);
				let (cq, cr) = world_layout::world_to_tile(px, py, w, h);
				let row = cr + cq / 2;
				let rt = (radius.0 as f32) / 100.0;
				let ri = rt as i64;
				let half = (f64::from(rt) * 0.5) as i64;
				for x in (cq - half - 3).max(0)..(cq + half + 4).min(w) {
					for y in (row - ri - 3).max(0)..(row + ri + 3).min(h) {
						let (q, r) = (x, y - x / 2);
						let d = ((q - cq).abs() + (r - cr).abs() + (q - cq + r - cr).abs()) / 2;
						if d <= ri + 3 {
							if let Some(id) = grid.idx(q, r + q / 2) {
								grid.tiles[id].is_discovered = true;
							}
						}
					}
				}
				grid.fog.stamp((px as f32, py as f32), rt);
			})
		};
		// Fog-of-war switches. setFogOfWar(false) reveals the whole map, as the original
		// does. (The player's view, `setPlayerPos`/`setPlayerVisionRadius`, lives with the
		// entities: it decides `isHiddenToPlayer`.)
		let is_using_fog_of_war = {
			let g = self.grid.clone();
			vm.make_closure(move || g.borrow().fog_on)
		};
		let set_fog_of_war = {
			let g = self.grid.clone();
			vm.make_closure(move |on: bool| {
				let mut g = g.borrow_mut();
				g.fog_on = on;
				if !on {
					for t in &mut g.tiles {
						t.is_discovered = true;
					}
				}
			})
		};
		let find_next_region = {
			let g = self.grid.clone();
			vm.make_closure(
				move |vm: &SQVM, tile: Retained, dist: i64, ty: i64| -> Result<i64, String> {
					let (q, r) = resolve_nav_coords(vm, &tile.0)
						.ok_or("findNextRegion: could not resolve tile")?;
					let grid = g.borrow();
					let found = world_search::flood_last(
						(q, r),
						dist,
						|q, r| grid.idx(q, r + q / 2).is_some(),
						|q, r| {
							let t = &grid.tiles[grid.idx(q, r + q / 2)?];
							(t.region != 0 && (ty == 0 || t.type_ == ty)).then_some(t.region)
						},
					);
					Ok(found.unwrap_or(0))
				},
			)
		};
		// World.worldToTile(pos): the axial {X=q, Y=r} of the tile containing a world
		// pixel position (consumed by getTile); exact original semantics, see world_layout.
		let world_to_tile = {
			let g = self.grid.clone();
			vm.make_closure(move |vm: &SQVM, pos: Retained| -> Result<SqStrongRef, String> {
				let px = pos.0.get_member::<Number>(vm, "X").map_or(0.0, |n| n.0);
				let py = pos.0.get_member::<Number>(vm, "Y").map_or(0.0, |n| n.0);
				let (w, h) = {
					let grid = g.borrow();
					(grid.width, grid.height)
				};
				let (q, r) = world_layout::world_to_tile(px, py, w, h);
				Ok(xy_table(vm, q, r))
			})
		};
		// World.findNextTileOfType(tile, dist, type, result): set result.Result to the
		// nearest interior tile of `type` (see tile_of_type); return whether one was found.
		let find_next_tile_of_type = {
			let g = self.grid.clone();
			vm.make_closure(
				move |vm: &SQVM,
				      tile: Retained,
				      dist: i64,
				      ty: i64,
				      mut result: SqTable|
				      -> Result<bool, String> {
					let (q, r) = resolve_nav_coords(vm, &tile.0)
						.ok_or("findNextTileOfType: could not resolve tile")?;
					let found = {
						let grid = g.borrow();
						super::tile_of_type::next_tile_of_type(
							(q, r),
							dist,
							ty,
							grid.tiles.len(),
							|q, r| grid.idx(q, r + q / 2),
							|id| (grid.tiles[id].q, grid.tiles[id].r),
							|id| grid.tiles[id].type_,
						)
					};
					match found.and_then(|id| {
						let (c, row) = {
							let grid = g.borrow();
							(grid.tiles[id].col, grid.tiles[id].row)
						};
						tile_at(&g, vm, c, row)
					}) {
						Some(t) => {
							result.set("Result", t);
							Ok(true)
						}
						None => Ok(false),
					}
				},
			)
		};
		// World.spawnRegionText(name, scale, rotation, pos): a region's name on the map,
		// kept for the renderer (`0x4f0350`; see `SceneRegionText`).
		let spawn_region_text = {
			let g = self.grid.clone();
			vm.make_closure(move |vm: &SQVM, name: SqBytes, scale: Number, rotation: Number, pos: Retained| {
				let coord = |axis: &str| pos.0.get_member::<Number>(vm, axis).map_or(0.0, |n| n.0 as f32);
				let mut g = g.borrow_mut();
				// It's a scene entity in the original, so it takes an id.
				let id = g.ids.next();
				g.region_texts.push(SceneRegionText {
					id,
					text: name.0,
					scale: scale.0 as f32,
					rotation: rotation.0 as f32,
					pos: (coord("X"), coord("Y")),
				});
			})
		};

		// World.getNavigator(): a shared navigator with createSettings() (mutable
		// {ActionPointCosts, RoadMult, StopAtRoad}) and findPath(from, to, settings,
		// maxLen) → a WorldNavigationPath (hex A*). render/update/getLastSettings are
		// no-ops (no rendering). Worldgen uses it to reject non-connectable settlement
		// tiles (TerrainTypeNavCost, maxLen 0); the road phases reuse it later.
		let navigator = {
			let create_settings = vm.make_closure(|vm: &SQVM| -> SqStrongRef {
				// The original's defaults (createSettings disassembly): every cost 2,
				// RoadMult 0.0 (roads are free unless a script sets it), HeatCost 0.
				let d = NavSettings::default();
				let mut costs = SqArray::new(vm);
				for c in d.costs {
					costs.push(i64::from(c));
				}
				let mut s = SqTable::new(vm);
				s.set("ActionPointCosts", costs);
				s.set("RoadMult", f64::from(d.road_mult));
				s.set("HeatCost", i64::from(d.heat_cost));
				s.set("StopAtRoad", d.stop_at_road);
				s.set("RoadOnly", d.road_only);
				s.to_strong()
			});
			let find_path = {
				let g = self.grid.clone();
				vm.make_closure(
					move |vm: &SQVM,
					      from: Retained,
					      to: Retained,
					      settings: SqTable,
					      max_len: i64|
					      -> Result<Option<SqStrongRef>, String> {
						// Resolve endpoints BEFORE borrowing the grid (resolving an entity
						// calls getTile → World.getTile, which borrows the grid).
						let from_c = resolve_nav_coords(vm, &from.0)
							.ok_or("findPath: could not resolve 'from'")?;
						let to_c = resolve_nav_coords(vm, &to.0)
							.ok_or("findPath: could not resolve 'to'")?;
						let nav = nav_settings(&settings);
						let tiles = {
							let grid = g.borrow();
							world_nav::find_path(from_c, to_c, max_len, &nav, |q, r| {
								grid.idx(q, r + q / 2).map(|id| {
									let t = &grid.tiles[id];
									NavTile {
										id,
										type_: usize::try_from(t.type_).unwrap_or(usize::MAX),
										road: t.road_directions != 0,
										heat: t.heat as u8,
									}
								})
							})
						};
						let class = g
							.borrow()
							.nav_path_class
							.clone()
							.ok_or("findPath: navigation-path class missing")?;
						Ok(Some(class.instantiate(
							vm,
							PathData {
								grid: g.clone(),
								tiles,
								cursor: 0,
							},
						)))
					},
				)
			};
			let nav_noop = vm.make_closure(|_a: Variadic<Retained>| {});
			let mut nav = SqTable::new(vm);
			nav.set("createSettings", create_settings);
			nav.set("findPath", find_path);
			nav.set("render", nav_noop.clone());
			nav.set("update", nav_noop.clone());
			nav.set("getLastSettings", nav_noop);
			nav.to_strong()
		};
		let get_navigator = {
			let nav = navigator.clone();
			vm.make_closure(move || nav.clone())
		};

		let tile_to_world = tile_to_world_overloads(vm);
		let mut world = SqTable::root(vm)
			.get::<SqTable>("World")
			.expect("World table must be installed before WorldGrid");
		world.set("resizeScene", resize);
		world.set("clearTiles", clear);
		world.set("getMapSize", get_map_size);
		world.set("getTileSquare", get_tile_square);
		world.set("getAllTilesOfRegion", get_all_tiles_of_region);
		world.set(&overload_key("getTile", 1), get_tile_1);
		world.set(&overload_key("getTile", 2), get_tile_2);
		world.set("getTile", get_tile_dispatch);
		world.set("isValidTileSquare", is_valid_square);
		world.set("isValidTile", is_valid_tile);
		world.set("getNumOfTilesWithType", num_of_type);
		world.set("queryTilesInRange", query_tiles_in_range);
		world.set("updateTilesWithHeat", update_tiles_with_heat);
		world.set("findNextRegion", find_next_region);
		world.set("uncoverFogOfWar", uncover_fog_of_war);
		world.set("isUsingFogOfWar", is_using_fog_of_war);
		world.set("setFogOfWar", set_fog_of_war);
		world.set("worldToTile", world_to_tile);
		for (key, closure) in tile_to_world {
			world.set(&key, closure);
		}
		world.set("findNextTileOfType", find_next_tile_of_type);
		world.set("spawnRegionText", spawn_region_text);
		world.set("getNavigator", get_navigator);
	}

	/// The grid dimensions `(width, height)` in square coords.
	pub fn dims(&self) -> (i64, i64) {
		let g = self.grid.borrow();
		(g.width, g.height)
	}

	/// [`dims`](Self::dims) for a host hook that outlives this borrow.
	pub(crate) fn dims_reader(&self) -> impl Fn() -> (i64, i64) + 'static {
		let grid = self.grid.clone();
		move || {
			let g = grid.borrow();
			(g.width, g.height)
		}
	}

	/// Read a tile's `Type` by square coords (host/test inspection).
	pub fn tile_type(&self, col: i64, row: i64) -> Option<i64> {
		let g = self.grid.borrow();
		g.idx(col, row).map(|id| g.tiles[id].type_)
	}

	/// Release the retained `WorldTile`/`WorldDetail` class refs. Call before the VM
	/// closes (breaks the Rust↔VM cycle; [`Engine`](crate::Engine) does this for you).
	/// The map's share of a save (see [`GridPart`]).
	pub(crate) fn save_part(&self) -> Rc<dyn super::save::SavePart> {
		Rc::new(GridPart(self.grid.clone()))
	}

	/// Every tile as the renderer sees it.
	pub fn scene_tiles(&self) -> Vec<crate::scene::SceneTile> {
		self.grid
			.borrow()
			.tiles
			.iter()
			.map(|t| crate::scene::SceneTile {
				col: t.col,
				row: t.row,
				pos: world_layout::tile_pos(t.col, t.row),
				terrain: t.type_,
				discovered: t.is_discovered,
				brush: t.brush.clone(),
				flipped: t.is_flipped,
				details: t.details.iter().map(|d| d.borrow().clone()).collect(),
			})
			.collect()
	}

	/// The brush that uncovering stamps into the fog mask (`fog_of_war_mask`).
	pub fn set_fog_stamp(&self, stamp: FogStamp) {
		self.grid.borrow_mut().fog.set_stamp(stamp);
	}

	/// The fog mask's texels changed since the last call, for a renderer to upload.
	pub fn take_fog_patch(&self) -> Option<FogPatch> {
		self.grid.borrow_mut().fog.take_patch()
	}

	/// The region names, in spawn order.
	pub fn region_texts(&self) -> Vec<SceneRegionText> {
		self.grid.borrow().region_texts.clone()
	}

	/// Fog of war is on (`World.setFogOfWar`).
	pub fn fog_of_war(&self) -> bool {
		self.grid.borrow().fog_on
	}

	/// The map's extent in world units (see [`fog_extent`]); `(0, 0)` with no map.
	pub fn extent(&self) -> (f32, f32) {
		let g = self.grid.borrow();
		if g.width > 0 && g.height > 0 { fog_extent(g.width, g.height) } else { (0.0, 0.0) }
	}

	pub fn clear(&self) {
		let mut g = self.grid.borrow_mut();
		g.class = None;
		g.detail_class = None;
		g.nav_path_class = None;
	}

	/// Turn the details with a torque to virtual time `now`, in seconds (see `spin`).
	pub(crate) fn turn(&self, now: f32) {
		let spinning = self.grid.borrow().spinning.clone();
		world_detail::turn_all(&spinning, now);
	}
}

/// Keep the camera's view between the centres of the tile at axial `(0, 0)` and the
/// one at `(w - 1, h - w / 2 - 1)`, as the original does when the map is sized or
/// loaded (`0x4ef610`).
fn bound_camera(g: &GridState) {
	let (Some(camera), true) = (&g.camera, g.width > 0 && g.height > 0) else { return };
	let (w, h) = (g.width, g.height);
	let (q, r) = (w - 1, h - w / 2 - 1);
	let min = world_layout::tile_pos(0, 0);
	let max = world_layout::tile_pos(q, r + q / 2);
	camera
		.borrow_mut()
		.set_bounds((min.0 as f32, min.1 as f32), (max.0 as f32, max.1 as f32));
}

/// The map's extent on a `w` × `h` map, which the fog mask and the water texture span:
/// the centre of the tile at axial `(w, h - w / 2)`, just past the last corner
/// (`0x4f4ce7`), truncated.
fn fog_extent(w: i64, h: i64) -> (f32, f32) {
	let (x, y) = world_layout::tile_pos(w, h);
	(x.trunc() as f32, y.trunc() as f32)
}

/// Mint a `WorldTile` instance at square `(col,row)`, or `None` if off-grid.
/// The world point halfway between two tiles' centres (NaN for a tile and itself).
fn between(a: &TileHandle, b: &TileHandle) -> (f32, f32) {
	let pa = a.with(|d| world_layout::tile_pos(d.col, d.row));
	let pb = b.with(|d| world_layout::tile_pos(d.col, d.row));
	world_layout::midpoint(pa, pb)
}

fn tile_at(grid: &Rc<RefCell<GridState>>, vm: &SQVM, col: i64, row: i64) -> Option<SqStrongRef> {
	let id = grid.borrow().idx(col, row)?;
	let class = grid.borrow().class.clone()?;
	Some(class.instantiate(
		vm,
		TileHandle {
			grid: grid.clone(),
			id,
		},
	))
}

/// Add a detail carrying `data` to the tile and mint its `WorldDetail` object, or
/// `None` if the detail class is not installed (it always is after `install`).
/// Fetches the class from the grid at call time so the tile methods capture only the
/// `Rc`, not an `SqStrongRef`.
///
/// Like the original (`0x4fb4d0`, `0x4fb5d0`), the engine's own random stream varies
/// the new detail. A form without a mirroring argument draws one at random. Either
/// way only a brush with flag `0x400` mirrors (settlement houses and forests don't).
/// A brush that rotates (or has flag `0x2000`) starts at a random rotation, and one
/// with flag `0x1000` (ground scatter: grass, bones, coast details) gets a random
/// scale in `[0.6, 1.0]`. Drawn in that order. A rotating brush's speed is the
/// detail's torque (see `spin`).
fn make_detail(t: &TileHandle, vm: &SQVM, mut data: DetailData, random_flip: bool, offset: (f32, f32)) -> Option<SqStrongRef> {
	let class = t.grid.borrow().detail_class.clone()?;
	let (sprites, rng, spinning, ids) = {
		let g = t.grid.borrow();
		(g.sprites.clone(), g.engine_rng.clone(), g.spinning.clone(), g.ids.clone())
	};
	if random_flip {
		data.flipped = rng.range(0, 1) == 0;
	}
	// An unknown brush spawns nothing (the original returns null; before brushes are
	// loaded every name is kept).
	if sprites.resolve(&data.brush).is_empty() {
		return None;
	}
	// Its id, place and draw order, as the original's decal gets them (`0x4fb5d0`).
	data.id = ids.next();
	let (col, row, r) = t.with(|d| (d.col, d.row, d.r));
	let (cx, cy) = world_layout::tile_pos(col, row);
	data.pos = (cx as f32 + offset.0, offset.1 + cy as f32);
	let depth_bias = sprites.with(|r| r.get(&data.brush).map_or(0, |(_, s)| i32::from(s.depth_bias)));
	data.z = (data.pos.1 as i32) + depth_bias + data.z_level as i32 - (r & 1) as i32;
	let looks = sprites.with(|r| r.get(&data.brush).map(|(_, s)| (s.flags, s.rotation_speed)));
	if let Some((flags, rotation_speed)) = looks {
		data.flipped &= flags & 0x400 != 0;
		if rotation_speed != 0.0 || flags & 0x2000 != 0 {
			data.rotation = rng.range(0, 359) as f64;
		}
		if flags & 0x1000 != 0 {
			data.scale = f64::from(rng.range(0, 40) as f32 / 100.0 + 0.6);
		}
		data.spin.torque = rotation_speed;
	}
	hint_brush(t, &data.brush);
	let detail: Detail = Rc::new(RefCell::new(data));
	world_detail::note_spin(&spinning, &detail);
	t.with_mut(|d| d.details.push(detail.clone()));
	Some(class.instantiate(vm, DetailHandle(detail)))
}

/// `World.tileToWorld(coords)` / `tileToWorld(x, y)`: an axial tile's centre in world
/// pixels, as a vector.
fn tile_to_world_overloads(vm: &mut SQVM) -> Vec<(String, SqStrongRef)> {
	fn centre(vm: &SQVM, q: i64, r: i64) -> Result<Option<SqStrongRef>, String> {
		let create_vec = SqTable::root(vm)
			.get::<SqStrongRef>("createVec")
			.ok_or("tileToWorld: createVec missing")?;
		create_vec.call_ret::<SqStrongRef, _>(vm, world_layout::tile_pos(q, r + q / 2))
	}
	let from_coords = vm.make_closure(|vm: &SQVM, coords: Retained| -> Result<Option<SqStrongRef>, String> {
		let q = coords.0.get_member::<i64>(vm, "X").ok_or("tileToWorld: coords need X")?;
		let r = coords.0.get_member::<i64>(vm, "Y").ok_or("tileToWorld: coords need Y")?;
		centre(vm, q, r)
	});
	let from_xy = vm.make_closure(|vm: &SQVM, q: i64, r: i64| centre(vm, q, r));
	let dispatch = make_dispatcher(vm, "tileToWorld");
	vec![
		(overload_key("tileToWorld", 1), from_coords),
		(overload_key("tileToWorld", 2), from_xy),
		("tileToWorld".to_string(), dispatch),
	]
}

/// Build the `WorldNavigationPath` class returned by `findPath`. `isEmpty()` powers the
/// connectivity check; `getCurrent()`/`pop()`/`getSize()` are the road-laying cursor
/// (the loop `while (path.getSize() >= 1) { getTile(path.getCurrent()); path.pop(); }`).
/// `getCurrent` returns the waypoint's axial `{X, Y}` coords (fed to `World.getTile`).
fn build_world_navigation_path_class(vm: &mut SQVM) -> SqClass<PathData> {
	ClassBuilder::<PathData>::new("WorldNavigationPath")
		.method("isEmpty", |p: &mut PathData| -> bool { p.cursor >= p.tiles.len() })
		.method("getSize", |p: &mut PathData| -> i64 {
			p.tiles.len().saturating_sub(p.cursor) as i64
		})
		.method(
			"getCurrent",
			|p: &mut PathData, vm: &SQVM| -> Option<SqStrongRef> {
				let id = *p.tiles.get(p.cursor)?;
				let (q, r) = {
					let g = p.grid.borrow();
					let t = &g.tiles[id];
					(t.q, t.r)
				};
				Some(xy_table(vm, q, r))
			},
		)
		.method("pop", |p: &mut PathData| {
			if p.cursor < p.tiles.len() {
				p.cursor += 1;
			}
		})
		// getNext and getNextNext are one function in the original: the second waypoint
		// if there are two or more, else the current one.
		.method("getNext", |p: &mut PathData, vm: &SQVM| -> Option<SqStrongRef> {
			let at = if p.tiles.len().saturating_sub(p.cursor) > 1 { p.cursor + 1 } else { p.cursor };
			p.coords_at(vm, at)
		})
		.method("getNextNext", |p: &mut PathData, vm: &SQVM| -> Option<SqStrongRef> {
			let at = if p.tiles.len().saturating_sub(p.cursor) > 1 { p.cursor + 1 } else { p.cursor };
			p.coords_at(vm, at)
		})
		.method("getLast", |p: &mut PathData, vm: &SQVM| -> Option<SqStrongRef> {
			if p.cursor >= p.tiles.len() {
				return None;
			}
			p.coords_at(vm, p.tiles.len() - 1)
		})
		.method("pushFront", |p: &mut PathData, vm: &SQVM, coords: Retained| {
			if let Some(id) = p.tile_of(vm, &coords.0) {
				p.tiles.insert(p.cursor, id);
			}
		})
		.method("pushBack", |p: &mut PathData, vm: &SQVM, coords: Retained| {
			if let Some(id) = p.tile_of(vm, &coords.0) {
				p.tiles.push(id);
			}
		})
		// isAtWaypoint(pos): within 4 px of the current waypoint's tile centre (an empty
		// path counts as arrived). From the original's disassembly.
		.method("isAtWaypoint", |p: &mut PathData, vm: &SQVM, pos: Retained| -> bool {
			let Some(&id) = p.tiles.get(p.cursor) else {
				return true;
			};
			let (q, r) = {
				let g = p.grid.borrow();
				(g.tiles[id].q, g.tiles[id].r)
			};
			let x = pos.0.get_member::<Number>(vm, "X").map_or(0.0, |n| n.0);
			let y = pos.0.get_member::<Number>(vm, "Y").map_or(0.0, |n| n.0);
			let centre = world_layout::tile_pos(q, r + q / 2);
			super::world_query::distance(centre, (x, y)) <= 4.0
		})
		.build(vm)
}


/// Read a script-side settings table the way the original's settings object stores it:
/// `ActionPointCosts` entries truncated to ints (numbers, not strict floats: the cost
/// tables mix an integer `0` with floats), at most 32, unset entries keeping the default.
fn nav_settings(t: &SqTable) -> NavSettings {
	let mut s = NavSettings::default();
	if let Some(costs) = t.get::<SqArray>("ActionPointCosts") {
		for (i, c) in costs.to_vec::<Number>().into_iter().take(32).enumerate() {
			s.costs[i] = world_nav::cvtt_i32(c.0);
		}
	}
	if let Some(m) = t.get::<Number>("RoadMult") {
		s.road_mult = m.0 as f32;
	}
	if let Some(h) = t.get::<Number>("HeatCost") {
		s.heat_cost = world_nav::cvtt_i32(h.0);
	}
	s.road_only = t.get::<bool>("RoadOnly").unwrap_or(false);
	s.stop_at_road = t.get::<bool>("StopAtRoad").unwrap_or(false);
	s
}

/// Resolve a `findPath` argument — a `WorldTile`, or an entity with `getTile()` — to its
/// axial `(q, r)`. Tiles answer the `X`/`Y` properties directly; an entity falls through
/// to its tile.
fn resolve_nav_coords(vm: &SQVM, arg: &SqStrongRef) -> Option<(i64, i64)> {
	if let (Some(q), Some(r)) = (
		arg.get_member::<i64>(vm, "X"),
		arg.get_member::<i64>(vm, "Y"),
	) {
		return Some((q, r));
	}
	let tile = arg
		.call_method_ret::<SqStrongRef, _>(vm, "getTile", ())
		.ok()
		.flatten()?;
	Some((tile.get_member::<i64>(vm, "X")?, tile.get_member::<i64>(vm, "Y")?))
}

/// Build the `WorldTile` native class: scalar fields + object-coord fields + the
/// construct-critical methods, with visual/detail methods stubbed.
fn build_world_tile_class(vm: &mut SQVM) -> SqClass<TileHandle> {
	macro_rules! scalar {
		($b:expr, $name:literal, $field:ident, $ty:ty) => {
			$b.property(
				$name,
				|t: &TileHandle| t.with(|d| d.$field),
				|t: &mut TileHandle, v: $ty| t.with_mut(|d| d.$field = v),
			)
		};
	}
	macro_rules! ro_scalar {
		($b:expr, $name:literal, |$t:ident| $get:expr, $ty:ty) => {
			$b.property(
				$name,
				|$t: &TileHandle| $get,
				|_t: &mut TileHandle, _v: $ty| {},
			)
		};
	}

	let mut b = ClassBuilder::<TileHandle>::new("WorldTile");
	// scalar rw fields
	b = scalar!(b, "Type", type_, i64);
	b = scalar!(b, "TacticalType", tactical_type, i64);
	b = scalar!(b, "Subregion", subregion, i64);
	b = scalar!(b, "Region", region, i64);
	b = scalar!(b, "RoadDirections", road_directions, i64);
	b = scalar!(b, "RiverDirections", river_directions, i64);
	b = scalar!(b, "HeatFromSettlements", heat, i64);
	b = scalar!(b, "ZOffset", z_offset, f64);
	b = scalar!(b, "IsOccupied", is_occupied, bool);
	b = scalar!(b, "IsFlipped", is_flipped, bool);
	b = scalar!(b, "IsDiscovered", is_discovered, bool);
	b = scalar!(b, "HasRiver", has_river, bool);
	// ro scalar fields
	b = ro_scalar!(b, "ID", |t| t.with(|d| d.id as i64), i64);
	b = ro_scalar!(b, "X", |t| t.with(|d| d.q), i64);
	b = ro_scalar!(b, "Y", |t| t.with(|d| d.r), i64);
	b = ro_scalar!(b, "HasRoad", |t| t.with(|d| d.road_directions != 0), bool);
	// object coord fields
	b = b
		.property_obj("Coords", |t, vm| {
			let (q, r) = t.with(|d| (d.q, d.r));
			xy_table(vm, q, r)
		})
		.property_obj("SquareCoords", |t, vm| {
			let (c, r) = t.with(|d| (d.col, d.row));
			xy_table(vm, c, r)
		})
		// The tile centre in world pixels, as a vec2f (like the original); see
		// `world_layout`.
		.property_obj("Pos", |t, vm| {
			let (c, r) = t.with(|d| (d.col, d.row));
			let (x, y) = world_layout::tile_pos(c, r);
			SqTable::root(vm)
				.get::<SqStrongRef>("createVec")
				.and_then(|create| create.call_ret::<SqStrongRef, _>(vm, (x, y)).ok().flatten())
				.unwrap_or_else(|| xy_table(vm, x as i64, y as i64))
		});

	// methods
	b = b
		.method(
			"getNextTile",
			|t: &mut TileHandle, vm: &SQVM, dir: i64| -> Option<SqStrongRef> {
				let id = t.neighbour_id(dir)?;
				let class = t.grid.borrow().class.clone()?;
				Some(class.instantiate(
					vm,
					TileHandle {
						grid: t.grid.clone(),
						id,
					},
				))
			},
		)
		.method("hasNextTile", |t: &mut TileHandle, dir: i64| -> bool {
			t.neighbour_id(dir).is_some()
		})
		// getDistanceTo(tile) or (x, y) in axial coords: Sqrat overloads.
		.method(
			&overload_key("getDistanceTo", 1),
			|t: &mut TileHandle, other: Native<TileHandle>| -> i64 { hex_distance(t, &other.0) },
		)
		.method(&overload_key("getDistanceTo", 2), |t: &mut TileHandle, x: i64, y: i64| -> i64 {
			hex_distance_to(t, x, y)
		})
		.native_method("getDistanceTo", {
			let overload = Overload::new("getDistanceTo");
			move |vm: &SQVM, this: This, args: StackArgs| overload.dispatch(vm, this, args)
		})
		.method(
			"isSameTileAs",
			|t: &mut TileHandle, other: Native<TileHandle>| -> bool { t.id == other.0.id },
		)
		// The point and the tile halfway between two tiles (see
		// `world_layout::midpoint`). A tile and itself divide by zero: NaN, which picks
		// tile 0,0 (probed).
		.method(
			"getPosBetweenThisAnd",
			|t: &mut TileHandle, vm: &SQVM, other: Native<TileHandle>| -> Option<SqStrongRef> {
				let (x, y) = between(t, &other.0);
				SqTable::root(vm)
					.get::<SqStrongRef>("createVec")
					.and_then(|create| create.call_ret::<SqStrongRef, _>(vm, (x as f64, y as f64)).ok().flatten())
			},
		)
		.method(
			"getTileBetweenThisAnd",
			|t: &mut TileHandle, vm: &SQVM, other: Native<TileHandle>| -> Option<SqStrongRef> {
				let (x, y) = between(t, &other.0);
				let (w, h) = {
					let g = t.grid.borrow();
					(g.width, g.height)
				};
				let (q, r) = world_layout::world_to_tile(x as f64, y as f64, w, h);
				tile_at(&t.grid, vm, q, r + q / 2)
			},
		)
		.method(
			"getSurroundingTilesOfType",
			|t: &mut TileHandle, ty: i64| -> i64 { surrounding(t, |d| d.type_ == ty) },
		)
		.method(
			"getSurroundingTilesNotOfType",
			|t: &mut TileHandle, ty: i64| -> i64 { surrounding(t, |d| d.type_ != ty) },
		)
		.method(
			"getDirectionTo",
			|t: &mut TileHandle, other: Native<TileHandle>| -> i64 { direction_to(t, &other.0) },
		)
		.method(
			"getDirection8To",
			|t: &mut TileHandle, other: Native<TileHandle>| -> i64 { direction8_to(t, &other.0) },
		);

	// `spawnDetail` is a Sqrat overload of arities 3/4/5/6 (disassembled, `0x4fb000`):
	// (brush, z, type) mirrors at random; then (…, flipped), (…, flipped, fadeIn) and
	// (…, flipped, fadeIn, offset). A fading-in detail is drawn at full alpha once
	// drawn, which is how a save keeps it, so the fade isn't modelled. The leaves add a
	// detail to the tile and mint its `WorldDetail`; the strict dispatcher routes by arg
	// count and is hookable per-overload. The trailing arguments are read leniently.
	b = b
		.method(
			&overload_key("spawnDetail", 3),
			|t: &mut TileHandle, vm: &SQVM, brush: &str, z: i64, flags: i64| -> Option<SqStrongRef> {
				make_detail(
					t,
					vm,
					DetailData {
						brush: brush.to_string(),
						z_level: z,
						flags,
						..DetailData::default()
					},
					true,
					(0.0, 0.0),
				)
			},
		)
		.method(
			&overload_key("spawnDetail", 4),
			|t: &mut TileHandle,
			 vm: &SQVM,
			 brush: &str,
			 z: Retained,
			 flags: Retained,
			 flipped: Retained|
			 -> Option<SqStrongRef> {
				make_detail(t, vm, detail_args(vm, brush, &z, &flags, &flipped), false, (0.0, 0.0))
			},
		)
		.method(
			&overload_key("spawnDetail", 5),
			|t: &mut TileHandle,
			 vm: &SQVM,
			 brush: &str,
			 z: Retained,
			 flags: Retained,
			 flipped: Retained,
			 _fade_in: Retained|
			 -> Option<SqStrongRef> {
				make_detail(t, vm, detail_args(vm, brush, &z, &flags, &flipped), false, (0.0, 0.0))
			},
		)
		.method(
			&overload_key("spawnDetail", 6),
			|t: &mut TileHandle,
			 vm: &SQVM,
			 brush: &str,
			 z: Retained,
			 flags: Retained,
			 flipped: Retained,
			 _fade_in: Retained,
			 offset: Retained|
			 -> Option<SqStrongRef> {
				let coord = |axis: &str| offset.0.get_member::<Number>(vm, axis).map_or(0.0, |n| n.0 as f32);
				let offset = (coord("X"), coord("Y"));
				make_detail(t, vm, detail_args(vm, brush, &z, &flags, &flipped), false, offset)
			},
		)
		.native_method(
			"spawnDetail",
			{
			let overload = Overload::new("spawnDetail");
			move |vm: &SQVM, this: This, args: StackArgs| overload.dispatch(vm, this, args)
		},
		);

	// The terrain brush is state worldgen reads back.
	b = b
		.method("setBrush", |t: &mut TileHandle, brush: &str| {
			t.with_mut(|d| d.brush = brush.to_string());
			hint_brush(t, brush);
		})
		.method("resetBrush", |t: &mut TileHandle| t.with_mut(|d| d.brush.clear()))
		.method("getBrush", |t: &mut TileHandle| -> String { t.with(|d| d.brush.clone()) });

	// Removing details. `clear()` removes all, `clear(mask)` those with any of the
	// type bits, `clearAllBut(mask)` those with none; `clearAndFade` removes like
	// `clear` (the fade is visual). `clear` and `clearAllBut` are Sqrat overloads
	// (`clear_0`/`clear_1`, `clearAllBut_1`). Arguments are read leniently.
	let clear = |t: &mut TileHandle, vm: &SQVM, args: Variadic<Retained>| clear_details(t, vm, &args.0, false);
	let clear_all_but = |t: &mut TileHandle, vm: &SQVM, args: Variadic<Retained>| clear_details(t, vm, &args.0, true);
	b = b
		.method(&overload_key("clear", 0), clear)
		.method(&overload_key("clear", 1), clear)
		.native_method("clear", {
			let overload = Overload::new("clear");
			move |vm: &SQVM, this: This, args: StackArgs| overload.dispatch(vm, this, args)
		})
		.method("clearAndFade", clear)
		.method(&overload_key("clearAllBut", 1), clear_all_but)
		.native_method("clearAllBut", {
			let overload = Overload::new("clearAllBut");
			move |vm: &SQVM, this: This, args: StackArgs| overload.dispatch(vm, this, args)
		})
		// hasDetailType(mask): whether one of the tile's details has any of the type
		// bits (0x4fbd10, which finds them through the scene around the tile).
		.method("hasDetailType", |t: &mut TileHandle, mask: i64| -> bool {
			t.with(|d| world_detail::has_type(&d.details, mask))
		});

	// `setColor` is a Sqrat overload (`setColor_1`, `setColor_3`); it and `onRoadPass`
	// stay lenient no-ops (worldgen never reads their result).
	let ignore = |_t: &mut TileHandle, _args: Variadic<Retained>| {};
	b = b
		.method(&overload_key("setColor", 1), ignore)
		.method(&overload_key("setColor", 3), ignore)
		.native_method("setColor", {
			let overload = Overload::new("setColor");
			move |vm: &SQVM, this: This, args: StackArgs| overload.dispatch(vm, this, args)
		})
		.method("onRoadPass", ignore);

	b.build(vm)
}

/// Hint the atlas of a brush just set on the tile or one of its details.
fn hint_brush(t: &TileHandle, brush: &str) {
	let sprites = t.grid.borrow().sprites.clone();
	sprites.hint(&sprites.texture_of(brush));
}

/// A detail from `spawnDetail`'s leniently read arguments (a missing or mistyped
/// argument keeps the default).
fn detail_args(vm: &SQVM, brush: &str, z: &Retained, flags: &Retained, flipped: &Retained) -> DetailData {
	let int = |arg: &Retained| arg.0.get_value::<i64>(vm);
	let mut data = DetailData {
		brush: brush.to_string(),
		..DetailData::default()
	};
	data.z_level = int(z).unwrap_or(data.z_level);
	data.flags = int(flags).unwrap_or(data.flags);
	data.flipped = flipped.0.get_value::<bool>(vm).unwrap_or(false);
	data
}

/// `clear`/`clearAllBut`: remove the tile's details selected by the optional mask.
fn clear_details(t: &TileHandle, vm: &SQVM, args: &[Retained], all_but: bool) {
	let mask = args.first().and_then(|a| a.0.get_value::<i64>(vm));
	let which = match (mask, all_but) {
		(None, _) => Clear::All,
		(Some(mask), false) => Clear::Matching(mask),
		(Some(mask), true) => Clear::AllBut(mask),
	};
	t.with_mut(|d| world_detail::clear(&mut d.details, which));
}

/// Count the 6 hex neighbours whose tile data satisfies `pred`.
fn surrounding(t: &TileHandle, pred: impl Fn(&TileData) -> bool) -> i64 {
	let g = t.grid.borrow();
	let (q, r) = {
		let d = &g.tiles[t.id];
		(d.q, d.r)
	};
	let mut count = 0;
	for (dq, dr) in DIRECTION_STEP {
		let (nq, nr) = (q + dq, r + dr);
		if let Some(id) = g.idx(nq, nr + nq / 2)
			&& pred(&g.tiles[id])
		{
			count += 1;
		}
	}
	count
}

/// `tile.getDirection8To(other)`: one of 8 compass directions between the two tiles'
/// centres. The original (`0x4fa990`) converts both tiles to world positions and uses
/// the same angle buckets as the entity version.
fn direction8_to(a: &TileHandle, b: &TileHandle) -> i64 {
	let pos = |t: &TileHandle| t.with(|d| world_layout::tile_pos(d.q, d.r + d.q / 2));
	super::world_query::direction8(pos(a), pos(b))
}

/// `WorldTile.getDirectionTo` as the original computes it (disassembled at `0x4daef0`;
/// matches 372 probed tile pairs): only the signs of the axial deltas count. N if the
/// columns match and `dr > 0`, else S (so a tile's direction to itself is S); NE or SE
/// to the east and NW or SW to the west, by the sign of `dr`. It is not the step that
/// gets closest to `b`.
fn direction_to(a: &TileHandle, b: &TileHandle) -> i64 {
	let (q, r) = a.with(|d| (d.q, d.r));
	let (bq, br) = b.with(|d| (d.q, d.r));
	let (dq, dr) = (bq - q, br - r);
	match dq.signum() {
		0 => if dr > 0 { 0 } else { 3 },
		1 => if dr < 0 { 2 } else { 1 },
		_ => if dr > 0 { 5 } else { 4 },
	}
}
