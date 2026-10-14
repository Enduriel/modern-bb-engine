//! `Tactical.queryTilesInRange` and `Tactical.queryActorsInRange`, as the original
//! filters them (disassembled), over the tiles of [`range`](super::range) in its order.
//!
//! - Actors: each tile in range holding an actor (not a map object) at least `min`
//!   away gives `callback(actor, userdata)`.
//! - Tiles: each tile in range that is passable, empty if asked, outside every zone of
//!   control of a faction not listed (no list: no zone check), with `TVTotal` at least
//!   the given floor (the 7-argument form: none), and at least `min` away gives
//!   `callback(tile, userdata)`.
//!
//! The centre is never included. No map borrow is held across a callback (it runs
//! script, which may touch the map).

use squirrel::{Native, Retained, SQVM, SqArray, SqStrongRef};

use super::zone::{FactionSet, faction_mask};
use super::{Grid, TacticalGridState, TileHandle, hex_distance, range, tile_handle};

/// "No floor" for `TVTotal`, as the 7-argument form passes it.
pub(crate) const NO_TV_FLOOR: i64 = -9000;

/// Whether `queryTilesInRange` reports tile `id` (`occupied` says whether a unit or
/// object stands on a tile; with `allied` factions listed, a zone of control from any
/// other faction excludes the tile).
#[allow(clippy::too_many_arguments)]
pub(crate) fn tile_passes(
	g: &TacticalGridState,
	centre: usize,
	id: usize,
	min: i64,
	only_empty: bool,
	tv_floor: i64,
	allied: Option<&FactionSet>,
	occupied: bool,
) -> bool {
	let t = &g.tiles[id];
	t.type_ != 0
		&& !(only_empty && occupied)
		&& allied.is_none_or(|allied| !t.zoc.has_outside(allied))
		&& t.tv_total >= tv_floor
		&& hex_distance(t, &g.tiles[centre]) >= min
}

/// Whether `queryActorsInRange` reports what stands on tile `id` (`is_actor`: whether
/// an actor, rather than an object, stands there).
pub(crate) fn actor_passes(g: &TacticalGridState, centre: usize, id: usize, min: i64, is_actor: Option<bool>) -> bool {
	is_actor == Some(true) && hex_distance(&g.tiles[id], &g.tiles[centre]) >= min
}

pub(super) fn query_actors(
	g: &Grid,
	vm: &SQVM,
	centre: usize,
	min: i64,
	max: i64,
	callback: &SqStrongRef,
	userdata: &SqStrongRef,
) -> Result<(), String> {
	let ids = range::tiles_in_range(&g.borrow(), centre, max);
	for id in ids {
		let actor = {
			let gb = g.borrow();
			let t = &gb.tiles[id];
			match &t.occupant {
				Some((e, is_actor)) if actor_passes(&gb, centre, id, min, Some(*is_actor)) => Some(e.clone()),
				_ => None,
			}
		};
		if let Some(actor) = actor {
			callback.call(vm, (actor, userdata.clone()))?;
		}
	}
	Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn query_tiles(
	g: &Grid,
	vm: &SQVM,
	centre: usize,
	min: i64,
	max: i64,
	only_empty: bool,
	tv_floor: i64,
	allied: &[i64],
	callback: &SqStrongRef,
	userdata: &SqStrongRef,
) -> Result<(), String> {
	let allied = (!allied.is_empty()).then(|| faction_mask(allied));
	let ids = range::tiles_in_range(&g.borrow(), centre, max);
	for id in ids {
		let take = {
			let gb = g.borrow();
			let occupied = gb.tiles[id].occupant.is_some();
			tile_passes(&gb, centre, id, min, only_empty, tv_floor, allied.as_ref(), occupied)
		};
		if take {
			let tile = tile_handle(g, vm, id).ok_or("queryTilesInRange: no tile class")?;
			callback.call(vm, (tile, userdata.clone()))?;
		}
	}
	Ok(())
}

/// The natives: `queryActorsInRange` and the two `queryTilesInRange` overloads.
pub(super) fn natives(vm: &mut SQVM, g: &Grid) -> [(String, SqStrongRef); 3] {
	let actors = {
		let g = g.clone();
		vm.make_closure(
			move |vm: &SQVM, centre: Native<TileHandle>, min: i64, max: i64, callback: SqStrongRef, data: Retained| {
				query_actors(&g, vm, centre.0.id, min, max, &callback, &data.0)
			},
		)
	};
	let tiles_7 = {
		let g = g.clone();
		vm.make_closure(
			move |vm: &SQVM,
			      centre: Native<TileHandle>,
			      min: i64,
			      max: i64,
			      only_empty: bool,
			      allied: SqArray,
			      callback: SqStrongRef,
			      data: Retained|
			      -> Result<(), String> {
				let allied = allied.to_vec::<i64>();
				query_tiles(&g, vm, centre.0.id, min, max, only_empty, NO_TV_FLOOR, &allied, &callback, &data.0)
			},
		)
	};
	let tiles_8 = {
		let g = g.clone();
		vm.make_closure(
			move |vm: &SQVM,
			      centre: Native<TileHandle>,
			      min: i64,
			      max: i64,
			      only_empty: bool,
			      tv_floor: i64,
			      allied: SqArray,
			      callback: SqStrongRef,
			      data: Retained|
			      -> Result<(), String> {
				let allied = allied.to_vec::<i64>();
				query_tiles(&g, vm, centre.0.id, min, max, only_empty, tv_floor, &allied, &callback, &data.0)
			},
		)
	};
	[
		("queryActorsInRange".to_string(), actors),
		(super::overload_key("queryTilesInRange", 7), tiles_7),
		(super::overload_key("queryTilesInRange", 8), tiles_8),
	]
}
