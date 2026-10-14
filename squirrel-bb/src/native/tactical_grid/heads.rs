//! Where a severed head lands, as the original chooses (`spawnHeadEffect`'s head,
//! `0x4bb390`; disassembled 2026-10-14):
//! - a neighbour of its tile is free for it when it's on the map, nobody stands on it,
//!   it's at most one level higher, and its properties show neither a head
//!   (`IsHeadSpawned`) nor a corpse with its head on (`Corpse.IsHeadAttached`);
//! - of the free ones, in the six directions' order, it takes `rand() % count`, one
//!   draw of the C runtime's stream (0-arg `Math.rand`'s);
//! - with none free it lands on its own tile, which is first cleared of severed heads
//!   (`clear(DetailFlag.SeveredHead)`) and of its `IsHeadSpawned`;
//! - the tile it lands on is marked `IsHeadSpawned` (`0x4baa49`).
//!
//! The original keeps a tile's properties in its `m` table, a corpse's under `Value`;
//! here they're the `Properties` object's `Flags` (see `tile`), the corpse itself.

use squirrel::{SQVM, SqRef, SqStrongRef};

use super::Grid;
use super::tile::{TileHandle, clear_tile, properties};

/// The property a landed head marks its tile with.
const HEAD_SPAWNED: &str = "IsHeadSpawned";

/// `DetailFlag.SeveredHead`.
const SEVERED_HEAD: i64 = 16;

/// Squirrel's truth of a value.
fn truthy(value: &SqRef) -> bool {
	match value {
		SqRef::Null => false,
		SqRef::Bool(b) => *b,
		SqRef::Integer(i) => *i != 0,
		SqRef::Float(f) => *f != 0.0,
		_ => true,
	}
}

/// Whether a head may land on the tile whose `Properties` are `props` (`None`: never
/// made, so empty).
fn free_for_a_head(vm: &SQVM, props: Option<&SqStrongRef>) -> bool {
	let Some(flags) = props.and_then(|p| p.get_member::<SqStrongRef>(vm, "Flags")).and_then(|f| f.as_table(vm)) else {
		return true;
	};
	if flags.get_ref(HEAD_SPAWNED).is_some_and(|v| !matches!(v, SqRef::Null)) {
		return false;
	}
	match flags.get_ref("Corpse") {
		Some(SqRef::Table(corpse)) => !corpse.get_ref("IsHeadAttached").is_some_and(|v| truthy(&v)),
		_ => true,
	}
}

/// The tile a head flying from tile `from` lands on. `rand` is the C runtime's
/// `rand()`, drawn once when there's a free neighbour to choose from.
pub(crate) fn landing_tile(grid: &Grid, vm: &SQVM, from: usize, rand: impl FnOnce() -> i64) -> usize {
	let free: Vec<usize> = {
		let g = grid.borrow();
		let level = g.tiles[from].level;
		(0..6)
			.filter_map(|dir| g.neighbour(from, dir))
			.filter(|&n| {
				let t = &g.tiles[n];
				t.occupant.is_none() && t.level <= level + 1 && free_for_a_head(vm, t.properties.as_ref())
			})
			.collect()
	};
	if free.is_empty() {
		let tile = TileHandle {
			grid: grid.clone(),
			id: from,
		};
		clear_tile(&tile, vm, Some(SEVERED_HEAD));
		let props = grid.borrow().tiles[from].properties.clone();
		if let Some(mut flags) = props.and_then(|p| p.get_member::<SqStrongRef>(vm, "Flags")).and_then(|f| f.as_table(vm)) {
			flags.remove(HEAD_SPAWNED);
		}
		return from;
	}
	free[rand() as usize % free.len()]
}

/// Mark tile `id` as holding a head.
pub(crate) fn mark_head(grid: &Grid, vm: &SQVM, id: usize) {
	let tile = TileHandle {
		grid: grid.clone(),
		id,
	};
	if let Some(mut flags) = props_flags(&tile, vm).and_then(|f| f.as_table(vm)) {
		flags.set(HEAD_SPAWNED, true);
	}
}

/// Tile `t`'s `Properties.Flags` (making its `Properties` if need be).
fn props_flags(t: &TileHandle, vm: &SQVM) -> Option<SqStrongRef> {
	properties(t, vm).get_member::<SqStrongRef>(vm, "Flags")
}

#[cfg(test)]
mod tests {
	use std::cell::{Cell, RefCell};
	use std::rc::Rc;

	use squirrel::{SQVM, SqTable};

	use super::*;
	use crate::native::tactical_grid::TacticalGridState;

	fn grid() -> Grid {
		Rc::new(RefCell::new(TacticalGridState::for_test(5, 5)))
	}

	/// Whether tile `id` is marked as holding a head.
	fn marked(grid: &Grid, vm: &SQVM, id: usize) -> bool {
		let props = grid.borrow().tiles[id].properties.clone();
		props
			.and_then(|p| p.get_member::<SqStrongRef>(vm, "Flags"))
			.and_then(|f| f.as_table(vm))
			.is_some_and(|f| f.contains(HEAD_SPAWNED))
	}

	#[test]
	fn a_head_takes_a_free_neighbour_by_one_draw() {
		let vm = SQVM::new(1 << 16);
		let g = grid();
		let from = g.borrow().idx(2, 2).unwrap();
		let around: Vec<usize> = (0..6).filter_map(|d| g.borrow().neighbour(from, d)).collect();
		assert_eq!(around.len(), 6);
		// Taken: someone stands there; two levels up; a head lies there; a corpse with
		// its head on. Free: the last two (one has a headless corpse).
		g.borrow_mut().tiles[around[0]].occupant = Some((SqTable::new(&vm).to_strong(), false));
		g.borrow_mut().tiles[around[1]].level = 2;
		mark_head(&g, &vm, around[2]);
		let corpse = |attached: bool| {
			let mut c = SqTable::new(&vm);
			c.set("IsHeadAttached", attached);
			c.to_strong()
		};
		for (tile, attached) in [(around[3], true), (around[4], false)] {
			let t = TileHandle { grid: g.clone(), id: tile };
			let mut flags = props_flags(&t, &vm).and_then(|f| f.as_table(&vm)).unwrap();
			flags.set("Corpse", corpse(attached));
		}
		let draws = Cell::new(0);
		let land = landing_tile(&g, &vm, from, || {
			draws.set(draws.get() + 1);
			3
		});
		assert_eq!(draws.get(), 1, "one draw of the runtime's rand()");
		assert_eq!(land, around[5], "3 % 2 = the second free one");
		let land = landing_tile(&g, &vm, from, || 4);
		assert_eq!(land, around[4], "4 % 2 = the first free one");
	}

	#[test]
	fn with_nowhere_to_go_it_lands_at_home() {
		let vm = SQVM::new(1 << 16);
		let g = grid();
		let from = g.borrow().idx(2, 2).unwrap();
		for d in 0..6 {
			let n = g.borrow().neighbour(from, d).unwrap();
			g.borrow_mut().tiles[n].occupant = Some((SqTable::new(&vm).to_strong(), true));
		}
		let cleared = Rc::new(RefCell::new(Vec::new()));
		let seen = cleared.clone();
		g.borrow_mut().clear_hook = Some(Rc::new(move |_vm, id, flags| seen.borrow_mut().push((id, flags))));
		mark_head(&g, &vm, from);
		let land = landing_tile(&g, &vm, from, || panic!("no draw without a choice"));
		assert_eq!(land, from);
		assert_eq!(*cleared.borrow(), vec![(from, Some(SEVERED_HEAD))], "its heads cleared");
		assert!(!marked(&g, &vm, from), "and its mark");
		mark_head(&g, &vm, land);
		assert!(marked(&g, &vm, from));
	}
}
