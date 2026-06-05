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
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::rc::Rc;

use squirrel::{
	ClassBuilder, Native, Retained, SQVM, SqArray, SqClass, SqStrongRef, SqTable, This, Variadic,
};

use super::overload::{dispatch_overload, make_dispatcher, overload_key};

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
}

/// A scene detail (decoration) minted by `tile.spawnDetail(...)`. Rendering is
/// deferred, so this only records the fields worldgen and scripts set/read on the
/// returned object (`Alpha`/`Torque`/… and `setBrush`/`setBrightness`/`setDirty`); it
/// has no visual effect yet. `Color` (a settable object-valued field) is deferred — it
/// needs a settable object property and is only used by tactical code, not worldgen.
#[derive(Clone, Default)]
struct DetailData {
	brush: String,
	// Recorded from spawnDetail for the deferred render/detail subsystem; nothing
	// reads them yet (worldgen only sets Alpha/Torque on the returned detail).
	#[allow(dead_code)]
	z_level: i64,
	#[allow(dead_code)]
	flags: i64,
	alpha: i64,
	torque: i64,
	saturation: f64,
	scale: f64,
	brightness: f64,
	ignore_ambient: bool,
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
	detail_class: Option<SqClass<DetailData>>,
	/// The navigation-path class, used by `World.getNavigator().findPath` to mint
	/// path handles. Set during `install`; released by `clear`.
	nav_path_class: Option<SqClass<PathData>>,
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
	let (aq, ar) = a.with(|d| (d.q, d.r));
	let (bq, br) = b.with(|d| (d.q, d.r));
	let (dq, dr) = (aq - bq, ar - br);
	(dq.abs() + dr.abs() + (dq + dr).abs()) / 2
}

/// The host-owned World hex tile grid service. Install into a VM (after the `World`
/// table exists), then `World.resizeScene(w,h)` allocates it. Holds the `WorldTile`
/// class (an `SqStrongRef`), so [`clear`](Self::clear) it before the VM closes
/// ([`Engine`](crate::Engine) does this automatically).
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
			})),
		}
	}

	/// Build the `WorldTile` class and add the grid natives to the existing `World`
	/// table. The `World` table must already be installed (it is — by the stateless
	/// native config).
	pub fn install(&self, vm: &mut SQVM) {
		// Build the detail class first so `spawnDetail` can mint instances (the tile
		// methods fetch it from the grid at call time, capturing no `SqStrongRef`).
		let detail_class = build_world_detail_class(vm);
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
		// for every tile whose hex distance from `center` is in [min, max], call
		// `callback(tile, userdata)`. The world callers pass a `bindenv`'d closure that
		// mutates `userdata` (e.g. tallying terrain types in a region); the return value
		// is unused. (Tactical.queryTilesInRange is a separate, overloaded native — its
		// 7/8-arg forms collect into a result list — and is added when tactical lands.)
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
					let (cq, cr) = center.0.with(|d| (d.q, d.r));
					// Walk the hex disk of radius `max` in axial space; skip the inner
					// rings closer than `min`. No grid borrow is held across the
					// callback (it runs arbitrary script that may re-enter the grid).
					for dq in -max..=max {
						let lo = (-max).max(-dq - max);
						let hi = max.min(-dq + max);
						for dr in lo..=hi {
							let dist = (dq.abs() + dr.abs() + (dq + dr).abs()) / 2;
							if dist < min {
								continue;
							}
							let (nq, nr) = (cq + dq, cr + dr);
							if let Some(tile) = tile_at(&g, vm, nq, nr + nq / 2) {
								callback.call(vm, (tile, userdata.0.clone()))?;
							}
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
		let find_next_region = {
			let g = self.grid.clone();
			vm.make_closure(
				move |vm: &SQVM, tile: Retained, dist: i64, ty: i64| -> Result<i64, String> {
					let (q, r) = resolve_nav_coords(vm, &tile.0)
						.ok_or("findNextRegion: could not resolve tile")?;
					let grid = g.borrow();
					Ok(closest_in_disk(&grid, q, r, dist, |t| {
						t.region != 0 && (ty == 0 || t.type_ == ty)
					})
					.map_or(0, |id| grid.tiles[id].region))
				},
			)
		};
		// World.worldToTile(pos): invert `tile.Pos` (structural square coords) back to a
		// tile's axial {X=q, Y=r} (consumed by getTile).
		let world_to_tile = {
			let g = self.grid.clone();
			vm.make_closure(move |vm: &SQVM, pos: Retained| -> Result<SqStrongRef, String> {
				let px = pos.0.get_member::<f64>(vm, "X").unwrap_or(0.0);
				let py = pos.0.get_member::<f64>(vm, "Y").unwrap_or(0.0);
				let col = px.round() as i64;
				let row = py.round() as i64;
				let _ = g.borrow(); // keep the capture meaningful / consistent
				Ok(xy_table(vm, col, row - col / 2)) // square -> axial
			})
		};
		// World.findNextTileOfType(tile, dist, type, result): set result.Result to the
		// nearest tile of `type` within `dist`; return whether one was found.
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
						closest_in_disk(&grid, q, r, dist, |t| t.type_ == ty)
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
		// World.spawnRegionText(name, scale, rotation, pos): a region map label — render
		// only, so a no-op for headless worldgen.
		let spawn_region_text = vm.make_closure(|_a: Variadic<Retained>| {});

		// World.getNavigator(): a shared navigator with createSettings() (mutable
		// {ActionPointCosts, RoadMult, StopAtRoad}) and findPath(from, to, settings,
		// maxLen) → a WorldNavigationPath (hex A*). render/update/getLastSettings are
		// no-ops (no rendering). Worldgen uses it to reject non-connectable settlement
		// tiles (TerrainTypeNavCost, maxLen 0); the road phases reuse it later.
		let navigator = {
			let create_settings = vm.make_closure(|vm: &SQVM| -> SqStrongRef {
				let mut s = SqTable::new(vm);
				// Pre-create every field the callers assign with `=` (a table `=` needs
				// the slot to exist; callers set ActionPointCosts always, RoadMult/
				// StopAtRoad in the road phases).
				s.set("ActionPointCosts", None::<i64>);
				s.set("RoadMult", 1.0);
				s.set("StopAtRoad", false);
				s.set("RoadOnly", false);
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
						let costs = settings
							.get::<SqArray>("ActionPointCosts")
							.map(|a| a.to_vec::<f64>())
							.unwrap_or_default();
						let road_mult = settings.get::<f64>("RoadMult").unwrap_or(1.0);
						let road_only = settings.get::<bool>("RoadOnly").unwrap_or(false);
						let full = {
							let grid = g.borrow();
							a_star(&grid, from_c, to_c, &costs, road_mult, road_only, max_len)
						};
						// a_star returns [start, .., goal]; the path's waypoints are the
						// tiles after the start (callers hold the start themselves).
						let tiles: Vec<usize> = full.into_iter().skip(1).collect();
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

		let mut world = SqTable::root(vm)
			.get::<SqTable>("World")
			.expect("World table must be installed before WorldGrid");
		world.set("resizeScene", resize);
		world.set("clearTiles", clear);
		world.set("getMapSize", get_map_size);
		world.set("getTileSquare", get_tile_square);
		world.set(&overload_key("getTile", 1), get_tile_1);
		world.set(&overload_key("getTile", 2), get_tile_2);
		world.set("getTile", get_tile_dispatch);
		world.set("isValidTileSquare", is_valid_square);
		world.set("isValidTile", is_valid_tile);
		world.set("getNumOfTilesWithType", num_of_type);
		world.set("queryTilesInRange", query_tiles_in_range);
		world.set("updateTilesWithHeat", update_tiles_with_heat);
		world.set("findNextRegion", find_next_region);
		world.set("worldToTile", world_to_tile);
		world.set("findNextTileOfType", find_next_tile_of_type);
		world.set("spawnRegionText", spawn_region_text);
		world.set("getNavigator", get_navigator);
	}

	/// The grid dimensions `(width, height)` in square coords.
	pub fn dims(&self) -> (i64, i64) {
		let g = self.grid.borrow();
		(g.width, g.height)
	}

	/// Read a tile's `Type` by square coords (host/test inspection).
	pub fn tile_type(&self, col: i64, row: i64) -> Option<i64> {
		let g = self.grid.borrow();
		g.idx(col, row).map(|id| g.tiles[id].type_)
	}

	/// Release the retained `WorldTile`/`WorldDetail` class refs. Call before the VM
	/// closes (breaks the Rust↔VM cycle; [`Engine`](crate::Engine) does this for you).
	pub fn clear(&self) {
		let mut g = self.grid.borrow_mut();
		g.class = None;
		g.detail_class = None;
		g.nav_path_class = None;
	}
}

/// Mint a `WorldTile` instance at square `(col,row)`, or `None` if off-grid.
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

/// Mint a `WorldDetail` instance carrying `data`, or `None` if the detail class is
/// not installed (it always is after `install`). Fetches the class from the grid at
/// call time so the tile methods capture only the `Rc`, not an `SqStrongRef`.
fn make_detail(t: &TileHandle, vm: &SQVM, data: DetailData) -> Option<SqStrongRef> {
	let class = t.grid.borrow().detail_class.clone()?;
	Some(class.instantiate(vm, data))
}

/// Build the `WorldDetail` native class returned by `tile.spawnDetail(...)`. Records
/// the fields scripts set on a detail; no visual effect yet (rendering is deferred).
fn build_world_detail_class(vm: &mut SQVM) -> SqClass<DetailData> {
	ClassBuilder::<DetailData>::new("WorldDetail")
		.property(
			"Alpha",
			|d: &DetailData| d.alpha,
			|d: &mut DetailData, v: i64| d.alpha = v,
		)
		.property(
			"Torque",
			|d: &DetailData| d.torque,
			|d: &mut DetailData, v: i64| d.torque = v,
		)
		.property(
			"Saturation",
			|d: &DetailData| d.saturation,
			|d: &mut DetailData, v: f64| d.saturation = v,
		)
		.property(
			"Scale",
			|d: &DetailData| d.scale,
			|d: &mut DetailData, v: f64| d.scale = v,
		)
		.property(
			"IgnoreAmbientColor",
			|d: &DetailData| d.ignore_ambient,
			|d: &mut DetailData, v: bool| d.ignore_ambient = v,
		)
		.method("setBrush", |d: &mut DetailData, brush: &str| {
			d.brush = brush.to_string()
		})
		.method("setBrightness", |d: &mut DetailData, v: f64| d.brightness = v)
		.method("setDirty", |_d: &mut DetailData, _flag: bool| {})
		.build(vm)
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
		.build(vm)
}

/// Axial cube distance between two axial coords (the hex metric / A* heuristic).
fn hex_dist_axial(aq: i64, ar: i64, bq: i64, br: i64) -> i64 {
	let (dq, dr) = (aq - bq, ar - br);
	(dq.abs() + dr.abs() + (dq + dr).abs()) / 2
}

/// The flat id of the closest tile within hex distance `dist` of axial `(cq, cr)` (the
/// center itself excluded) whose data satisfies `pred`, or `None`. Backs `findNextRegion`
/// and `findNextTileOfType`.
fn closest_in_disk(
	g: &GridState,
	cq: i64,
	cr: i64,
	dist: i64,
	pred: impl Fn(&TileData) -> bool,
) -> Option<usize> {
	let mut best: Option<(i64, usize)> = None;
	for dq in -dist..=dist {
		let lo = (-dist).max(-dq - dist);
		let hi = dist.min(-dq + dist);
		for dr in lo..=hi {
			let d = (dq.abs() + dr.abs() + (dq + dr).abs()) / 2;
			if d == 0 || d > dist {
				continue;
			}
			let (nq, nr) = (cq + dq, cr + dr);
			if let Some(id) = g.idx(nq, nr + nq / 2)
				&& pred(&g.tiles[id])
				&& best.is_none_or(|(bd, _)| d < bd)
			{
				best = Some((d, id));
			}
		}
	}
	best.map(|(_, id)| id)
}

/// Hex A* over the grid: the min-cost path of tile ids from axial `from` to `to`,
/// honoring per-terrain `costs` (**0 == impassable**, e.g. Ocean) scaled by `road_mult`
/// on road tiles, bounded to `max_len` steps (0 == unbounded). Empty if unreachable.
/// Structural (BFS-of-costs) correctness; exact C++ tie-breaking is deferred with the
/// PRNG/seed work.
fn a_star(
	g: &GridState,
	from: (i64, i64),
	to: (i64, i64),
	costs: &[f64],
	road_mult: f64,
	road_only: bool,
	max_len: i64,
) -> Vec<usize> {
	let start = match g.idx(from.0, from.1 + from.0 / 2) {
		Some(i) => i,
		None => return Vec::new(),
	};
	let goal = match g.idx(to.0, to.1 + to.0 / 2) {
		Some(i) => i,
		None => return Vec::new(),
	};
	if start == goal {
		return vec![start];
	}

	// Scaled i64 cost to ENTER tile `id`; None if impassable (cost 0, e.g. Ocean).
	let enter_cost = |id: usize| -> Option<i64> {
		let t = &g.tiles[id];
		let base = costs.get(t.type_ as usize).copied().unwrap_or(0.0);
		if base <= 0.0 {
			return None;
		}
		let c = if t.road_directions != 0 {
			base * road_mult
		} else {
			base
		};
		Some((c * 1000.0).round().max(1.0) as i64)
	};
	let min_cost = costs
		.iter()
		.copied()
		.filter(|&c| c > 0.0)
		.fold(f64::INFINITY, f64::min);
	let h_unit = if min_cost.is_finite() {
		(min_cost * 1000.0).round().max(1.0) as i64
	} else {
		1
	};

	let n = g.tiles.len();
	let mut g_score = vec![i64::MAX; n];
	let mut came = vec![usize::MAX; n];
	let mut depth = vec![0i32; n];
	g_score[start] = 0;
	let mut heap: BinaryHeap<Reverse<(i64, usize)>> = BinaryHeap::new();
	heap.push(Reverse((
		hex_dist_axial(from.0, from.1, to.0, to.1) * h_unit,
		start,
	)));

	while let Some(Reverse((_f, cur))) = heap.pop() {
		if cur == goal {
			let mut path = vec![goal];
			let mut c = goal;
			while c != start {
				c = came[c];
				path.push(c);
			}
			path.reverse();
			return path;
		}
		let (cq, cr) = {
			let t = &g.tiles[cur];
			(t.q, t.r)
		};
		for (dq, dr) in DIRECTION_STEP {
			let (nq, nr) = (cq + dq, cr + dr);
			let nid = match g.idx(nq, nr + nq / 2) {
				Some(i) => i,
				None => continue,
			};
			// RoadOnly: only road tiles are traversable (the goal endpoint is always
			// allowed, since a settlement tile may anchor a road without carrying one).
			if road_only && nid != goal && g.tiles[nid].road_directions == 0 {
				continue;
			}
			let step = match enter_cost(nid) {
				Some(c) => c,
				None => continue, // impassable
			};
			let nd = depth[cur] + 1;
			if max_len > 0 && i64::from(nd) > max_len {
				continue;
			}
			let tentative = g_score[cur].saturating_add(step);
			if tentative < g_score[nid] {
				g_score[nid] = tentative;
				came[nid] = cur;
				depth[nid] = nd;
				let h = hex_dist_axial(nq, nr, to.0, to.1) * h_unit;
				heap.push(Reverse((tentative.saturating_add(h), nid)));
			}
		}
	}
	Vec::new()
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
		// Structural world position: the square coords as a stand-in for pixels (real
		// pixel layout / tile size is deferred). Invertible with `World.worldToTile`, so
		// the region-label centroid math is meaningful even without real pixels.
		.property_obj("Pos", |t, vm| {
			let (c, r) = t.with(|d| (d.col, d.row));
			xy_table(vm, c, r)
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
		.method(
			"getDistanceTo",
			|t: &mut TileHandle, other: Native<TileHandle>| -> i64 { hex_distance(t, &other.0) },
		)
		.method(
			"isSameTileAs",
			|t: &mut TileHandle, other: Native<TileHandle>| -> bool { t.id == other.0.id },
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
		);

	// `spawnDetail` is a Sqrat overload (arities 1/3/4/5/6 — from verified call sites).
	// The leaves mint a `WorldDetail`; the strict dispatcher routes by arg count and is
	// hookable per-overload. Worldgen uses the 3-arg form (fog/decoration whose
	// `Alpha`/`Torque` get set); other arities take their trailing args permissively
	// (rendering is deferred, so only brush/z/flags are recorded).
	b = b
		.method(
			&overload_key("spawnDetail", 1),
			|t: &mut TileHandle, vm: &SQVM, brush: &str| -> Option<SqStrongRef> {
				make_detail(
					t,
					vm,
					DetailData {
						brush: brush.to_string(),
						..DetailData::default()
					},
				)
			},
		)
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
				)
			},
		)
		.method(
			&overload_key("spawnDetail", 4),
			|t: &mut TileHandle,
			 vm: &SQVM,
			 brush: &str,
			 _a: Retained,
			 _b: Retained,
			 _c: Retained|
			 -> Option<SqStrongRef> {
				make_detail(
					t,
					vm,
					DetailData {
						brush: brush.to_string(),
						..DetailData::default()
					},
				)
			},
		)
		.method(
			&overload_key("spawnDetail", 5),
			|t: &mut TileHandle,
			 vm: &SQVM,
			 brush: &str,
			 z: i64,
			 flags: i64,
			 _flip: Retained,
			 _offset: Retained|
			 -> Option<SqStrongRef> {
				make_detail(
					t,
					vm,
					DetailData {
						brush: brush.to_string(),
						z_level: z,
						flags,
						..DetailData::default()
					},
				)
			},
		)
		.method(
			&overload_key("spawnDetail", 6),
			|t: &mut TileHandle,
			 vm: &SQVM,
			 brush: &str,
			 z: i64,
			 flags: i64,
			 _flip: Retained,
			 _extra: Retained,
			 _offset: Retained|
			 -> Option<SqStrongRef> {
				make_detail(
					t,
					vm,
					DetailData {
						brush: brush.to_string(),
						z_level: z,
						flags,
						..DetailData::default()
					},
				)
			},
		)
		.native_method(
			"spawnDetail",
			|vm: &SQVM, this: This, args: Variadic<Retained>| {
				dispatch_overload(vm, this, "spawnDetail", args)
			},
		);

	// visual / detail methods — no-op stubs (the render/detail subsystem is later).
	// `clear`/`clearAllBut`/`setColor` are themselves Sqrat overloads; they stay lenient
	// no-op stubs until the detail subsystem lands (worldgen never reads their result).
	for name in [
		"clear",
		"clearAllBut",
		"clearAndFade",
		"setBrush",
		"resetBrush",
		"getBrush",
		"setColor",
		"onRoadPass",
	] {
		b = b.method(name, |_t: &mut TileHandle, _args: Variadic<Retained>| {});
	}

	b.build(vm)
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

/// The hex direction (0..6) whose single step most reduces distance to `b` (the
/// movement direction); `0` if already on `b`.
fn direction_to(a: &TileHandle, b: &TileHandle) -> i64 {
	if hex_distance(a, b) == 0 {
		return 0;
	}
	let (q, r) = a.with(|d| (d.q, d.r));
	let (bq, br) = b.with(|d| (d.q, d.r));
	let mut best = 0i64;
	let mut best_dist = i64::MAX;
	for (i, (dq, dr)) in DIRECTION_STEP.iter().enumerate() {
		let (ddq, ddr) = (q + dq - bq, r + dr - br);
		let dist = (ddq.abs() + ddr.abs() + (ddq + ddr).abs()) / 2;
		if dist < best_dist {
			best_dist = dist;
			best = i as i64;
		}
	}
	best
}
