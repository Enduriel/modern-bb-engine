//! Putting tactical entities (units and map objects) on the battle map: the tile they
//! stand on, `getTile`/`isPlacedOnMap`/`removeFromMap`, and `tile.spawnObject`.

use std::cell::RefCell;
use std::rc::Rc;

use squirrel::{SQVM, SqStrongRef};

use super::{TacticalMachine, create_unit, ignore_missing};
use crate::native::tactical_grid::{self, ObjectSpawner};

type Machine = Rc<RefCell<TacticalMachine>>;

/// Put unit `idx` on tile `tile` (an actor or an object) as the original's set-tile does:
/// leave the old tile (if it was ours), claim the new one only if it's free (a second
/// entity on a tile doesn't displace the first), and stand at the tile's centre (local
/// position 0, which follows the tile's level).
pub(super) fn place(m: &Machine, idx: usize, tile: usize, is_actor: bool) -> Result<(), String> {
	let mut machine = m.borrow_mut();
	let grid = machine.grid.clone().ok_or("no battle map")?;
	let entity = machine.units[idx].entity.clone();
	let old = machine.units[idx].tile;
	if old == Some(tile) {
		return Ok(());
	}
	{
		let mut g = grid.borrow_mut();
		if let Some(o) = old {
			release(&mut g.tiles[o], &entity);
		}
		let t = g.tiles.get_mut(tile).ok_or("place: tile is off the map")?;
		if t.occupant.is_none() {
			t.occupant = Some((entity, is_actor));
		}
	}
	let rec = &mut machine.units[idx];
	rec.tile = Some(tile);
	rec.local = (0.0, 0.0);
	// Its sprites turn again (see `moving_sprites`).
	machine.moving.note(machine.owner(idx));
	Ok(())
}

/// Clear `t`'s occupant if it is `entity`.
fn release(t: &mut tactical_grid::TacticalTileData, entity: &SqStrongRef) {
	if t.occupant.as_ref().is_some_and(|(e, _)| e.identity() == entity.identity()) {
		t.occupant = None;
	}
}

/// Take unit `idx` off the map: its tile is freed (if it held it) and it stands at
/// (0, 0), like an unplaced entity in the original.
pub(super) fn unplace(m: &Machine, idx: usize) {
	let mut machine = m.borrow_mut();
	let Some(tile) = machine.units[idx].tile.take() else {
		return;
	};
	machine.units[idx].local = (0.0, 0.0);
	let entity = machine.units[idx].entity.clone();
	if let Some(grid) = machine.grid.clone() {
		if let Some(t) = grid.borrow_mut().tiles.get_mut(tile) {
			release(t, &entity);
		}
	}
}

/// `Tactical.clearScene()`'s share: every unit off the map, without its
/// `onRemovedFromMap` (probed 2026-10-14: the brothers who come back from a battle are
/// no longer placed once the battle's state has cleared the scene, and the callback
/// doesn't run; the original's clearScene, `0x4d3240`, empties the scene's entity list).
pub(super) fn unplace_all(m: &Machine) {
	let count = m.borrow().units.len();
	for idx in 0..count {
		unplace(m, idx);
	}
}

/// `Tactical.clearScene()`'s other share: the records of the units no roster holds (the
/// battle's enemies, its dead, its map objects) go, with their script objects, sprites
/// and labels; the rosters' (the company, guests, the towns' recruits) stay. Without it
/// every battle left ~50 records alive until a load or a quit to the menu. Call with
/// no shake running (`shaker::clear` ends them).
pub(super) fn release_unrostered(m: &Machine) {
	let mut machine = m.borrow_mut();
	let rostered: std::collections::HashSet<i64> = machine.rosters.values().flatten().map(SqStrongRef::identity).collect();
	let keep: Vec<bool> = machine.units.iter().map(|r| rostered.contains(&r.entity.identity())).collect();
	if keep.iter().all(|&k| k) {
		return;
	}
	let mut new_index = vec![usize::MAX; keep.len()];
	let mut next = 0;
	for (i, &k) in keep.iter().enumerate() {
		if k {
			new_index[i] = next;
			next += 1;
		}
	}
	machine.by_identity.retain(|_, idx| keep[*idx]);
	for idx in machine.by_identity.values_mut() {
		*idx = new_index[*idx];
	}
	let mut i = 0;
	machine.units.retain(|_| {
		i += 1;
		keep[i - 1]
	});
}

/// `entity.removeFromMap()`: run its `onRemovedFromMap` while still placed (it reads
/// its tile to lift its zones), then take it off. A no-op when not placed.
pub(super) fn remove_from_map(m: &Machine, vm: &SQVM, idx: usize) -> Result<(), String> {
	let (placed, entity) = {
		let machine = m.borrow();
		(machine.units[idx].tile.is_some(), machine.units[idx].entity.clone())
	};
	if !placed {
		return Ok(());
	}
	ignore_missing(entity.call_method(vm, "onRemovedFromMap", ()))?;
	unplace(m, idx);
	Ok(())
}

/// `Tactical.addEntityToMap(entity, x, y)`: place an existing entity (a brother from
/// the roster) at axial `(x, y)` and run its `onPlacedOnMap`. Already placed: the
/// original only logs an error.
pub(super) fn add_to_map(m: &Machine, vm: &SQVM, entity: &SqStrongRef, q: i64, r: i64) -> Result<(), String> {
	let (idx, placed, is_actor, grid) = {
		let machine = m.borrow();
		let idx = *machine
			.by_identity
			.get(&super::unit_identity(vm, entity))
			.ok_or("addEntityToMap: not a tactical entity")?;
		let rec = &machine.units[idx];
		(idx, rec.tile.is_some(), rec.is_actor, machine.grid.clone().ok_or("no battle map")?)
	};
	if placed {
		eprintln!("TacticalScene: Entity already placed on tactical map!");
		return Ok(());
	}
	let tile = grid.borrow().idx_axial(q, r).ok_or_else(|| format!("addEntityToMap: ({q}, {r}) is off the map"))?;
	place(m, idx, tile, is_actor)?;
	ignore_missing(entity.call_method(vm, "onPlacedOnMap", ()))
}

/// `Tactical.spawnEntity(path, …)` at tile `tile`: create the entity, put it on the
/// tile, run `onInit` (and `onAfterInit`), then `onPlacedOnMap`. Spawning onto an
/// occupied tile only warns (the occupant keeps the tile). A missing script gives
/// `null`. `parent` is the 4-argument form's entity id (the lindwurm tail's body), 0
/// otherwise; it becomes the entity's `m.ParentID` before `onInit`.
pub(super) fn spawn_at(m: &Machine, vm: &SQVM, path: &str, tile: usize, parent: i64) -> Result<Option<SqStrongRef>, String> {
	let id = m.borrow().ids.next();
	let (entity, idx) = match create_unit(m, vm, &script_path(path), id) {
		Ok(created) => created,
		Err(e) => {
			eprintln!("TacticalScene: failed to spawn {path}: {e}");
			return Ok(None);
		}
	};
	let (is_actor, occupied, coords) = {
		let machine = m.borrow();
		let grid = machine.grid.clone().ok_or("no battle map")?;
		let g = grid.borrow();
		let t = &g.tiles[tile];
		(machine.units[idx].is_actor, t.occupant.is_some(), (t.q, t.r))
	};
	if occupied {
		eprintln!("TacticalScene: Spawned Entity at ({},{}) although the tile is already occupied.", coords.0, coords.1);
	}
	super::set_parent_id(vm, &entity, parent);
	place(m, idx, tile, is_actor)?;
	entity.call_method(vm, "onInit", ())?;
	ignore_missing(entity.call_method(vm, "onAfterInit", ()))?;
	m.borrow_mut().units[idx].initialized = true;
	ignore_missing(entity.call_method(vm, "onPlacedOnMap", ()))?;
	Ok(Some(entity))
}

/// The tile handle unit `idx` stands on, if placed.
pub(super) fn tile_of(m: &Machine, vm: &SQVM, idx: usize) -> Option<SqStrongRef> {
	let (grid, tile) = {
		let machine = m.borrow();
		(machine.grid.clone()?, machine.units[idx].tile?)
	};
	tactical_grid::tile_handle(&grid, vm, tile)
}

/// A script path as `this.new` takes it (`spawnObject` paths omit `scripts/`).
pub(super) fn script_path(path: &str) -> String {
	if path.starts_with("scripts/") {
		path.to_string()
	} else {
		format!("scripts/{path}")
	}
}

/// What `tile.spawnObject(path)` runs: the same spawn as `Tactical.spawnEntity`.
pub(super) fn object_spawner(m: &Machine) -> ObjectSpawner {
	let m = m.clone();
	Rc::new(move |vm: &SQVM, path: &str, tile: usize| -> Result<SqStrongRef, String> {
		spawn_at(&m, vm, path, tile, 0)?.ok_or_else(|| format!("spawnObject: no script {path}"))
	})
}
