//! The units' side of battle visibility (see `tactical_grid::visibility`):
//! `updateVisibility`, `isHiddenToPlayer`, `isVisibleToEntity`, `setDiscovered`, and
//! `Tactical.clearVisibility`/`fillVisibility`. What a vision update causes runs after
//! its walk, in the original's order: discovered units' `onDiscovered`, then the
//! viewer's `onActorSighted(actor)` for each actor it saw.

use std::cell::RefCell;
use std::rc::Rc;

use squirrel::{Retained, SQVM, SqStrongRef};

use super::{TacticalMachine, ignore_missing};
use crate::native::tactical_grid::visibility::{Occupant, PLAYER, Sightings};
use crate::native::tactical_grid::{self, zone::faction_bit};

type Machine = Rc<RefCell<TacticalMachine>>;

/// What the walk needs to know about each tile's occupant.
pub(super) fn occupants(m: &Machine) -> Vec<Option<Occupant>> {
	let machine = m.borrow();
	let Some(grid) = machine.grid.clone() else { return Vec::new() };
	let g = grid.borrow();
	g.tiles
		.iter()
		.map(|t| {
			let (e, is_actor) = t.occupant.as_ref()?;
			let rec = machine.record(e.identity());
			Some(Occupant {
				identity: e.identity(),
				is_actor: *is_actor,
				blocks_sight: rec.is_some_and(|r| r.blocks_sight),
				discovered: rec.is_some_and(|r| r.discovered),
			})
		})
		.collect()
}

/// Record and announce what a vision update caused; `viewer` gets the sightings.
fn apply(m: &Machine, vm: &SQVM, s: Sightings, viewer: Option<&SqStrongRef>) -> Result<(), String> {
	for &id in &s.discovered {
		let entity = {
			let mut machine = m.borrow_mut();
			machine.record_mut(id).map(|r| {
				r.discovered = true;
				r.entity.clone()
			})
		};
		if let Some(e) = entity {
			ignore_missing(e.call_method(vm, "onDiscovered", ()))?;
		}
	}
	if let Some(viewer) = viewer {
		for &id in &s.sighted {
			let actor = m.borrow().record(id).map(|r| r.entity.clone());
			if let Some(actor) = actor {
				ignore_missing(viewer.call_method(vm, "onActorSighted", (actor,)))?;
			}
		}
	}
	Ok(())
}

/// The unit's tile and the map, if it's placed.
fn placed(m: &Machine, identity: i64) -> Option<(tactical_grid::Grid, usize)> {
	let machine = m.borrow();
	Some((machine.grid.clone()?, machine.record(identity)?.tile?))
}

/// The injected unit natives.
pub(super) fn unit_natives(vm: &mut SQVM, m: &Machine) -> Vec<(&'static str, SqStrongRef)> {
	// updateVisibility(tile, vision, faction): what the unit (as `faction`) sees from
	// `tile`.
	let update_visibility = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM, tile: Retained, vision: i64, faction: i64| -> Result<(), String> {
			let grid = m.borrow().grid.clone().ok_or("updateVisibility: no battle map")?;
			let from = tactical_grid::tile_id_of(&grid, vm, &tile.0).ok_or("updateVisibility: expected a tile")?;
			let mut occ = occupants(&m);
			let sightings = grid.borrow_mut().update_visibility(&mut occ, from, vision, faction);
			let viewer = m.borrow().record(vm.this_identity()).map(|r| r.entity.clone());
			apply(&m, vm, sightings, viewer.as_ref())
		})
	};
	// Placed: hidden unless the player sees its tile. Off the map: never.
	let is_hidden_to_player = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM| -> bool {
			placed(&m, vm.this_identity()).is_some_and(|(g, t)| g.borrow().tiles[t].vis_mask & faction_bit(PLAYER) == 0)
		})
	};
	// Whether the latest vision update (by anyone) saw the unit's tile.
	let is_visible_to_entity = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM| -> bool {
			placed(&m, vm.this_identity()).is_some_and(|(g, t)| {
				let g = g.borrow();
				g.tiles[t].vis_stamp == g.vis_stamp
			})
		})
	};
	// setDiscovered(true) on an undiscovered unit runs its onDiscovered.
	let set_discovered = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM, v: bool| -> Result<(), String> {
			let newly = {
				let mut machine = m.borrow_mut();
				match machine.record_mut(vm.this_identity()) {
					Some(r) => {
						let newly = v && !r.discovered;
						r.discovered = v;
						newly.then(|| r.entity.clone())
					}
					None => None,
				}
			};
			match newly {
				Some(e) => ignore_missing(e.call_method(vm, "onDiscovered", ())),
				None => Ok(()),
			}
		})
	};
	vec![
		("updateVisibility", update_visibility),
		("isHiddenToPlayer", is_hidden_to_player),
		("isVisibleToEntity", is_visible_to_entity),
		("setDiscovered", set_discovered),
	]
}

/// `Tactical.clearVisibility` and `Tactical.fillVisibility`.
pub(super) fn tactical_natives(vm: &mut SQVM, m: &Machine) -> Vec<(&'static str, SqStrongRef)> {
	let clear = {
		let m = m.clone();
		vm.make_closure(move || -> Result<(), String> {
			let grid = m.borrow().grid.clone().ok_or("clearVisibility: no battle map")?;
			grid.borrow_mut().clear_visibility();
			Ok(())
		})
	};
	let fill = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM, faction: i64, visible: bool| -> Result<(), String> {
			let grid = m.borrow().grid.clone().ok_or("fillVisibility: no battle map")?;
			let mut occ = occupants(&m);
			let sightings = grid.borrow_mut().fill_visibility(&mut occ, faction, visible);
			apply(&m, vm, sightings, None)
		})
	};
	vec![("clearVisibility", clear), ("fillVisibility", fill)]
}
