//! The tactical navigator (`Tactical.getNavigator()`): path search over the battle map
//! ([`pathfind`]), what a path costs a unit ([`cost`]), and the shared search settings
//! ([`settings`]). All reproduced from the original's natives.
//!
//! The current path lives here: `findPath` replaces it, `clipPathToDistance`,
//! `popLastWaypoint` and `clearPath` trim it, and the cost queries read it. Units move
//! along it with `travel` ([`travel`]) or straight to a tile with `teleport`
//! ([`teleport`]), and swap places with `switchEntities`.
//!
//! Host-owned like the other services: it holds script references, so
//! [`clear`](TacticalNavigator::clear) it before the VM closes.

mod cost;
#[cfg(test)]
mod ground_truth;
pub(crate) mod heap;
mod hop;
pub use hop::BattleMotion;
mod pathfind;
mod settings;
mod teleport;
mod travel;
mod visual;

use std::cell::RefCell;
use std::rc::Rc;

use squirrel::{ClassBuilder, Native, Number, Retained, SQVM, SqClass, SqStrongRef, SqTable, StackArgs, This, Variadic};

use crate::native::TacticalGrid;
use crate::native::overload::{Overload, expect_arity, overload_key};
use crate::native::tactical_entities::{Units, unit_identity};
use crate::native::tactical_grid::{self, Grid, TileHandle};
use pathfind::{Query, Step};
use settings::{NavSettings, SettingsHandle};

struct NavState {
	grid: Grid,
	units: Units,
	/// The move in progress (`travel`), if any.
	travel: Option<travel::Travel>,
	/// Teleports in progress, in the order they started.
	teleports: Vec<teleport::Teleport>,
	settings: Rc<RefCell<NavSettings>>,
	path: Vec<Step>,
	settings_class: Option<SqClass<SettingsHandle>>,
	/// The one `Tactical.getNavigator()` instance.
	instance: Option<SqStrongRef>,
	/// How moves are paced (the host's choice).
	motion: BattleMotion,
	/// The path's footsteps (`buildVisualisation`).
	markers: Vec<visual::Marker>,
}

impl NavState {
	fn settings_handle(&self, vm: &SQVM) -> Option<SqStrongRef> {
		let class = self.settings_class.clone()?;
		Some(class.instantiate(vm, SettingsHandle(self.settings.clone())))
	}

	/// The settings behind a script handle.
	fn settings_of(&self, vm: &SQVM, value: &SqStrongRef) -> Option<NavSettings> {
		let class = self.settings_class.clone()?;
		class.with_payload(vm, value, |h| h.0.borrow().clone())
	}

	/// The faction of whoever stands on each tile.
	fn occupants(&self) -> Vec<Option<i64>> {
		let g = self.grid.borrow();
		g.tiles
			.iter()
			.map(|t| {
				t.occupant
					.as_ref()
					.map(|(e, _)| self.units.info(e.identity()).map_or(0, |u| u.faction))
			})
			.collect()
	}

	/// The tile a placed unit stands on (`entity` may be a `WeakTableRef` to it).
	fn unit_tile(&self, vm: &SQVM, entity: &SqStrongRef) -> Option<usize> {
		self.units.info(unit_identity(vm, entity))?.tile
	}
}

/// A script handle to the navigator.
#[derive(Clone)]
struct NavHandle(Rc<RefCell<NavState>>);

pub struct TacticalNavigator {
	state: Rc<RefCell<NavState>>,
}

impl TacticalNavigator {
	pub(crate) fn new(grid: &TacticalGrid, units: Units) -> Self {
		TacticalNavigator {
			state: Rc::new(RefCell::new(NavState {
				grid: grid.handle(),
				units,
				travel: None,
				settings: Rc::new(RefCell::new(NavSettings::default())),
				path: Vec::new(),
				settings_class: None,
				instance: None,
				teleports: Vec::new(),
				motion: BattleMotion::default(),
				markers: Vec::new(),
			})),
		}
	}

	/// The path's footsteps as drawn now.
	pub(crate) fn scene_path(&self) -> Vec<crate::scene::tactical::ScenePathStep> {
		visual::scene(&self.state.borrow())
	}

	/// How units' moves are paced (see [`BattleMotion`]).
	pub fn set_motion(&self, motion: BattleMotion) {
		self.state.borrow_mut().motion = motion;
	}

	/// Build the navigator and settings classes and set `Tactical.getNavigator`. Call
	/// once, after the `Tactical` table exists.
	pub fn install(&self, vm: &mut SQVM) {
		let grid = self.state.borrow().grid.clone();
		let settings_class = settings::build_class(vm, grid);
		self.state.borrow_mut().settings_class = Some(settings_class);
		let class = build_class(vm);
		let instance = class.instantiate(vm, NavHandle(self.state.clone()));
		self.state.borrow_mut().instance = Some(instance.clone());
		let get = vm.make_closure(move || instance.clone());
		// Tactical.update(): the original updates the navigator (teleports in flight), the
		// camera shaker and the weather; only the first is logic.
		let update = {
			let st = self.state.clone();
			vm.make_closure(move |vm: &SQVM| -> Result<(), String> { teleport::update(&st, vm) })
		};
		let mut tactical = SqTable::root(vm)
			.get::<SqTable>("Tactical")
			.expect("Tactical table must be installed before TacticalNavigator");
		tactical.set("getNavigator", get);
		tactical.set("update", update);
	}

	/// Release every script reference. Call before the VM is closed.
	pub fn clear(&self) {
		let mut st = self.state.borrow_mut();
		st.instance = None;
		st.settings_class = None;
		st.path.clear();
		st.travel = None;
		st.teleports.clear();
	}
}

/// `getCostForPath`'s result when there's no unit on the map or no path.
fn empty_cost(vm: &SQVM) -> SqStrongRef {
	let mut t = SqTable::new(vm);
	for k in ["ActionPoints", "Fatigue", "ActionPointsRequired", "FatigueRequired", "Tiles"] {
		t.set(k, 0i64);
	}
	for k in ["IsComplete", "IsTraversable", "IsMissingActionPoints", "IsMissingFatigue"] {
		t.set(k, false);
	}
	for k in ["First", "End", "LastBeforeEnd"] {
		t.set(k, 0i64);
	}
	t.to_strong()
}

fn cost_table(vm: &SQVM, grid: &Grid, c: &cost::PathCost) -> SqStrongRef {
	let tile = |id: usize| tactical_grid::tile_handle(grid, vm, id);
	let mut t = SqTable::new(vm);
	t.set("ActionPoints", c.ap as i64);
	t.set("Fatigue", c.fatigue as i64);
	t.set("ActionPointsRequired", c.ap_required as i64);
	t.set("FatigueRequired", c.fatigue_required as i64);
	t.set("Tiles", c.tiles as i64);
	t.set("IsComplete", c.is_complete);
	t.set("IsTraversable", false);
	t.set("IsMissingActionPoints", c.missing_ap);
	t.set("IsMissingFatigue", c.missing_fatigue);
	t.set("First", tile(c.first));
	t.set("End", tile(c.end));
	t.set("LastBeforeEnd", tile(c.last_before_end));
	t.set("SecondLastBeforeEnd", tile(c.second_last_before_end));
	t.to_strong()
}

fn build_class(vm: &mut SQVM) -> SqClass<NavHandle> {
	ClassBuilder::<NavHandle>::new("TacticalNavigator")
		// The current path's steps (probed: `findPath`, `clipPathToDistance`,
		// `popLastWaypoint` and `clearPath` all keep it current; 0 after a failed search).
		.property_ro("NumPathWaypoints", |n: &NavHandle| n.0.borrow().path.len() as i64)
		.method("createSettings", |n: &mut NavHandle, vm: &SQVM| -> Option<SqStrongRef> {
			let st = n.0.borrow();
			*st.settings.borrow_mut() = NavSettings::default();
			st.settings_handle(vm)
		})
		.method("getLastSettings", |n: &mut NavHandle, vm: &SQVM| -> Option<SqStrongRef> { n.0.borrow().settings_handle(vm) })
		.method(
			"findPath",
			|n: &mut NavHandle, from: Native<TileHandle>, to: Native<TileHandle>, s: Native<SettingsHandle>, within: i64| -> bool {
				let mut st = n.0.borrow_mut();
				// The original refuses (keeping the path) while a unit is moving.
				if st.travel.is_some() {
					return false;
				}
				st.path.clear();
				let occupants = st.occupants();
				let settings = s.0.0.borrow().clone();
				let path = {
					let g = st.grid.borrow();
					let q = Query {
						grid: &g,
						occupant: &occupants,
						settings: &settings,
					};
					pathfind::find_path(&q, from.0.id, to.0.id, within as i32)
				};
				match path {
					Some(p) => {
						st.path = p;
						true
					}
					None => false,
				}
			},
		)
		.method("clearPath", |n: &mut NavHandle| n.0.borrow_mut().path.clear())
		.method("popLastWaypoint", |n: &mut NavHandle| {
			n.0.borrow_mut().path.pop();
		})
		// Keep the leading steps within `d` of `tile`; cut from the first one beyond.
		.method("clipPathToDistance", |n: &mut NavHandle, tile: Native<TileHandle>, d: i64| {
			let st = &mut *n.0.borrow_mut();
			pathfind::clip_to_distance(&st.grid.borrow(), &mut st.path, tile.0.id, d);
		})
		.method(
			"getCostForPath",
			|n: &mut NavHandle, vm: &SQVM, entity: Retained, s: Native<SettingsHandle>, ap: i64, fatigue: i64| -> SqStrongRef {
				let st = n.0.borrow();
				let Some(from) = st.unit_tile(vm, &entity.0) else { return empty_cost(vm) };
				if st.path.is_empty() {
					return empty_cost(vm);
				}
				let occupants = st.occupants();
				let settings = s.0.0.borrow().clone();
				let c = {
					let g = st.grid.borrow();
					cost::cost_for_path(&g, |i| occupants[i].is_some(), &st.path, &settings, from, ap as i32, fatigue as i32)
				};
				cost_table(vm, &st.grid, &c)
			},
		)
		// getTurnsRequiredForPath(unit, settings, ap) or (unit, settings, ap, target tile,
		// faction): Sqrat overloads.
		.method(
			&overload_key("getTurnsRequiredForPath", 3),
			|n: &mut NavHandle, vm: &SQVM, Variadic(args): Variadic<Retained>| -> Result<f64, String> {
				expect_arity(args.len(), 3)?;
				turns_required(n, vm, &args)
			},
		)
		.method(
			&overload_key("getTurnsRequiredForPath", 5),
			|n: &mut NavHandle, vm: &SQVM, Variadic(args): Variadic<Retained>| -> Result<f64, String> {
				expect_arity(args.len(), 5)?;
				turns_required(n, vm, &args)
			},
		)
		.native_method("getTurnsRequiredForPath", {
			let overload = Overload::new("getTurnsRequiredForPath");
			move |vm: &SQVM, this: This, args: StackArgs| overload.dispatch(vm, this, args)
		})
		// isTravelling(entity): whether any unit is moving along a path (the original asks
		// that, whoever the entity), or that one is teleporting.
		.method("isTravelling", |n: &mut NavHandle, vm: &SQVM, entity: Retained| -> bool {
			let id = unit_identity(vm, &entity.0);
			let st = n.0.borrow();
			st.travel.is_some() || st.teleports.iter().any(|t| t.identity == id)
		})
		.property_ro("HasValidPath", |n: &NavHandle| !n.0.borrow().path.is_empty())
		.property_ro("IsTravelling", |n: &NavHandle| {
			let st = n.0.borrow();
			st.travel.is_some() || !st.teleports.is_empty()
		})
		// travel(entity, ap, fatigue): start or continue moving along the path (see
		// `travel`); true while the move is under way.
		.method("travel", |n: &mut NavHandle, vm: &SQVM, entity: Retained, _ap: Number, _fatigue: Number| -> Result<bool, String> {
			travel::travel(&n.0, vm, &entity.0)
		})
		// teleport(entity, tile, onDone, tag, flag[, speed]) (see `teleport`): Sqrat overloads.
		.method(
			&overload_key("teleport", 5),
			|n: &mut NavHandle, vm: &SQVM, Variadic(args): Variadic<Retained>| -> Result<bool, String> {
				expect_arity(args.len(), 5)?;
				teleport::teleport(&n.0, vm, &args)
			},
		)
		.method(
			&overload_key("teleport", 6),
			|n: &mut NavHandle, vm: &SQVM, Variadic(args): Variadic<Retained>| -> Result<bool, String> {
				expect_arity(args.len(), 6)?;
				teleport::teleport(&n.0, vm, &args)
			},
		)
		.native_method("teleport", {
			let overload = Overload::new("teleport");
			move |vm: &SQVM, this: This, args: StackArgs| overload.dispatch(vm, this, args)
		})
		// switchEntities(a, b, onDone, tag, speed) (see `teleport`).
		.method("switchEntities", |n: &mut NavHandle, vm: &SQVM, Variadic(args): Variadic<Retained>| -> Result<bool, String> {
			teleport::switch_entities(&n.0, vm, &args)
		})
		// The path overlay is visual.
		// buildVisualisation(entity, settings, ap, fatigue): the path's footsteps (see
		// `visual`); the costs are the navigator's own settings, not `settings`.
		.method(
			"buildVisualisation",
			|n: &mut NavHandle, vm: &SQVM, entity: Retained, _settings: Retained, ap: Number, fatigue: Number| {
				let mut st = n.0.borrow_mut();
				st.markers.clear();
				if let Some(start) = st.unit_tile(vm, &entity.0) {
					visual::build(&mut st, start, ap.0 as i64, fatigue.0 as i64);
				}
			},
		)
		.method("clearVisualisation", |n: &mut NavHandle| n.0.borrow_mut().markers.clear())
		.method("render", |_n: &mut NavHandle, _a: Variadic<Retained>| {})
		.method("update", |_n: &mut NavHandle, _a: Variadic<Retained>| {})
		.build(vm)
}

/// `getTurnsRequiredForPath` with 3 or 5 arguments (see the overloads above).
fn turns_required(n: &NavHandle, vm: &SQVM, args: &[Retained]) -> Result<f64, String> {
	let st = n.0.borrow();
	let settings = st.settings_of(vm, &args[1].0).ok_or("getTurnsRequiredForPath: expected settings")?;
	let ap = args[2].0.get_value::<Number>(vm).ok_or("getTurnsRequiredForPath: expected a number")?.0 as i32;
	let target = if args.len() == 5 {
		let tile = tactical_grid::tile_id_of(&st.grid, vm, &args[3].0).ok_or("getTurnsRequiredForPath: expected a tile")?;
		let faction = args[4].0.get_value::<Number>(vm).ok_or("getTurnsRequiredForPath: expected a faction")?.0 as i64;
		Some((tile, faction))
	} else {
		None
	};
	let Some(from) = st.unit_tile(vm, &args[0].0) else { return Ok(0.0) };
	if st.path.is_empty() {
		return Ok(0.0);
	}
	let g = st.grid.borrow();
	Ok(cost::turns_required(&g, &st.path, &settings, from, ap, target) as f64)
}
