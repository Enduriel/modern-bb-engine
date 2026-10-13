//! The `TacticalTile` script class: one battle tile's fields and methods.

use squirrel::{ClassBuilder, Native, Number, Retained, SQVM, SqArray, SqClass, SqStrongRef, SqTable, Variadic};

use super::{Grid, TacticalTileData, detail, hex_distance, tile_handle, vec};
use crate::native::world_query;

/// A script handle to one tile (a shared map + the tile's index).
#[derive(Clone)]
pub(crate) struct TileHandle {
	pub(crate) grid: Grid,
	pub(crate) id: usize,
}

impl TileHandle {
	fn with<R>(&self, f: impl FnOnce(&TacticalTileData) -> R) -> R {
		f(&self.grid.borrow().tiles[self.id])
	}
	fn with_mut<R>(&self, f: impl FnOnce(&mut TacticalTileData) -> R) -> R {
		f(&mut self.grid.borrow_mut().tiles[self.id])
	}
	fn pos(&self) -> (f64, f64) {
		self.with(TacticalTileData::world_pos)
	}
}

/// The methods every tile's `Properties` object delegates to: a small flag store
/// (`set`/`add`/`get`/`has`/`remove`), next to the plain `Effect` and
/// `IsMarkedForImpact` fields scripts read and write directly.
///
/// The tile's `IsCorpseSpawned` is whether its `Properties` hold a `Corpse` (probed: set
/// by `set("Corpse", …)` or `add("Corpse")`, even to null; cleared by `remove`; untouched
/// by `tile.clear()`), so those calls report the `Corpse` key to the map through
/// `set_corpse(tile, on)`.
pub(super) fn properties_delegate(vm: &mut SQVM, grid: &Grid) -> SqStrongRef {
	let src = r#"
	return {
		set = function( _key, _value ) { this.Flags[_key] <- _value; if (_key == "Corpse") this.__setCorpse(this.__tile, true); },
		add = function( _key ) { this.Flags[_key] <- true; if (_key == "Corpse") this.__setCorpse(this.__tile, true); },
		get = function( _key ) { return _key in this.Flags ? this.Flags[_key] : null; },
		has = function( _key ) { return _key in this.Flags; },
		remove = function( _key ) { if (_key in this.Flags) delete this.Flags[_key]; if (_key == "Corpse") this.__setCorpse(this.__tile, false); }
	};
	"#;
	let set_corpse = {
		let g = grid.clone();
		vm.make_closure(move |id: i64, on: bool| {
			let mut g = g.borrow_mut();
			if let Some(t) = usize::try_from(id).ok().and_then(|i| g.tiles.get_mut(i)) {
				t.is_corpse_spawned = on;
			}
		})
	};
	let f = vm.compile(src, "tile_properties").expect("tile properties delegate compiles");
	let delegate = f.call_ret::<SqStrongRef, _>(vm, ()).ok().flatten().expect("tile properties delegate builds");
	if let Some(mut d) = delegate.as_table(vm) {
		d.set("__setCorpse", set_corpse);
	}
	delegate
}

/// `tile.Properties`, made on first use.
fn properties(t: &TileHandle, vm: &SQVM) -> SqStrongRef {
	if let Some(p) = t.with(|d| d.properties.clone()) {
		return p;
	}
	let delegate = t.grid.borrow().properties_delegate.clone();
	let mut table = SqTable::new(vm);
	table.set("Effect", Option::<SqStrongRef>::None);
	table.set("IsMarkedForImpact", false);
	table.set("Flags", SqTable::new(vm).to_strong());
	table.set("__tile", t.id as i64);
	if let Some(d) = delegate.and_then(|d| d.as_table(vm)) {
		table.set_delegate(&d);
	}
	let p = table.to_strong();
	t.with_mut(|d| d.properties = Some(p.clone()));
	p
}

/// `tile.Items`, made on first use.
fn items(t: &TileHandle, vm: &SQVM) -> SqStrongRef {
	if let Some(i) = t.with(|d| d.items.clone()) {
		return i;
	}
	let a = SqArray::new(vm).to_strong();
	t.with_mut(|d| d.items = Some(a.clone()));
	a
}

/// The mask of a script array of faction ids.
fn factions(list: &SqArray) -> u64 {
	super::zone::faction_mask(&list.to_vec::<i64>())
}

macro_rules! rw {
	($b:expr, $name:literal, $field:ident, $ty:ty) => {
		$b.property(
			$name,
			|t: &TileHandle| t.with(|d| d.$field),
			|t: &mut TileHandle, v: $ty| t.with_mut(|d| d.$field = v),
		)
	};
}

macro_rules! ro {
	($b:expr, $name:literal, $get:expr, $ty:ty) => {
		$b.property($name, $get, |_t: &mut TileHandle, _v: $ty| {})
	};
}

pub(super) fn build_class(vm: &mut SQVM) -> SqClass<TileHandle> {
	let mut b = ClassBuilder::<TileHandle>::new("TacticalTile");
	b = rw!(b, "Level", level, i64);
	b = rw!(b, "Type", type_, i64);
	b = rw!(b, "Subtype", subtype, i64);
	b = rw!(b, "BlendPriority", blend_priority, i64);
	b = rw!(b, "IsBadTerrain", is_bad_terrain, bool);
	b = rw!(b, "IsSpecialTerrain", is_special_terrain, bool);
	b = rw!(b, "IsDefensibleTerrain", is_defensible_terrain, bool);
	b = rw!(b, "IsHidingEntity", is_hiding_entity, bool);
	// Read-only: it follows `Properties` (see `properties_delegate`).
	b = ro!(b, "IsCorpseSpawned", |t: &TileHandle| t.with(|d| d.is_corpse_spawned), bool);
	b = rw!(b, "IsDiscovered", is_discovered, bool);
	b = rw!(b, "IsContainingItemsFlipped", is_containing_items_flipped, bool);
	b = ro!(b, "ID", |t: &TileHandle| t.id as i64, i64);
	// The axial coords (`Coords.X`/`Coords.Y`) as plain fields.
	b = ro!(b, "X", |t: &TileHandle| t.with(|d| d.q), i64);
	b = ro!(b, "Y", |t: &TileHandle| t.with(|d| d.r), i64);
	b = ro!(b, "TVHeight", |t: &TileHandle| t.with(|d| d.tv_height), i64);
	b = ro!(b, "TVTerrain", |t: &TileHandle| t.with(|d| d.tv_terrain), i64);
	b = ro!(b, "TVLevelDisadvantage", |t: &TileHandle| t.with(|d| d.tv_level_disadvantage), i64);
	b = ro!(b, "TVTotal", |t: &TileHandle| t.with(|d| d.tv_total), i64);
	b = ro!(b, "IsVisibleForPlayer", |t: &TileHandle| t.with(|d| d.vis_mask & super::zone::faction_bit(1) != 0), bool);
	b = ro!(
		b,
		"IsVisibleForEntity",
		|t: &TileHandle| {
			let g = t.grid.borrow();
			g.tiles[t.id].vis_stamp == g.vis_stamp
		},
		bool
	);
	b = ro!(b, "IsEmpty", |t: &TileHandle| t.with(|d| d.occupant.is_none()), bool);
	b = ro!(b, "IsOccupiedByActor", |t: &TileHandle| t.with(|d| d.occupant.as_ref().is_some_and(|o| o.1)), bool);
	b = b
		.property_obj("Coords", |t, vm| {
			let (q, r) = t.with(|d| (d.q, d.r));
			xy(vm, q, r)
		})
		.property_obj("SquareCoords", |t, vm| {
			let (c, r) = t.with(|d| (d.col, d.row));
			xy(vm, c, r)
		})
		.property_obj("Pos", |t, vm| vec(vm, t.pos()).ok().flatten().unwrap_or_else(|| xy(vm, 0, 0)))
		.property_obj("Properties", properties)
		.property_obj("Items", items);
	// Set by the item scripts as items are dropped and picked up.
	b = rw!(b, "IsContainingItems", is_containing_items, bool);

	b = b
.method("hasLineOfSightTo", |t: &mut TileHandle, target: Native<TileHandle>, range: i64| -> bool {
			let lookup = t.grid.borrow().occupant_lookup.clone();
			let occupants = lookup.map_or_else(|| vec![None; t.grid.borrow().tiles.len()], |f| f());
			t.grid.borrow_mut().line_of_sight(&occupants, t.id, target.0.id, range)
		})
		.method("addVisibilityForFaction", |t: &mut TileHandle, f: i64| t.grid.borrow_mut().add_visibility(t.id, f))
		.method("addVisibilityForCurrentEntity", |t: &mut TileHandle| {
			let mut g = t.grid.borrow_mut();
			let stamp = g.vis_stamp;
			g.tiles[t.id].vis_stamp = stamp;
		})
		.method("getEntity", |t: &mut TileHandle| -> Option<SqStrongRef> {
			t.with(|d| d.occupant.as_ref().map(|o| o.0.clone()))
		})
		.method("spawnObject", |t: &mut TileHandle, vm: &SQVM, path: String| -> Result<SqStrongRef, String> {
			let spawner = t.grid.borrow().object_spawner.clone().ok_or("spawnObject: no entity host")?;
			spawner(vm, &path, t.id)
		})
		.method("removeObject", |t: &mut TileHandle| {
			t.with_mut(|d| {
				if d.occupant.as_ref().is_some_and(|o| !o.1) {
					d.occupant = None;
				}
			})
		})
		.method("setBrush", |t: &mut TileHandle, brush: String| t.with_mut(|d| d.brush = brush))
		.method("setColor", |_t: &mut TileHandle, _a: Variadic<Retained>| {})
		// clear(): every detail; clear(flags): those carrying any of `flags`.
		.method("clear", |t: &mut TileHandle, vm: &SQVM, Variadic(args): Variadic<Retained>| {
			let flags = args.first().and_then(|a| a.0.get_value::<Number>(vm)).map(|n| n.0 as i64);
			t.with_mut(|d| match flags {
				None => d.detail_flags.clear(),
				Some(f) => d.detail_flags.retain(|df| df & f == 0),
			})
		})
		// spawnDetail(brush [, flags [, flipped [, ? [, offset]]]]): a decal handle.
		.method("spawnDetail", |t: &mut TileHandle, vm: &SQVM, Variadic(args): Variadic<Retained>| -> Option<SqStrongRef> {
			let flags = args.get(1).and_then(|a| a.0.get_value::<Number>(vm)).map_or(0, |n| n.0 as i64);
			t.with_mut(|d| d.detail_flags.push(flags));
			let class = t.grid.borrow().detail_class.clone()?;
			Some(class.instantiate(vm, detail::DetailData::fresh()))
		})
		.method("hasNextTile", |t: &mut TileHandle, dir: i64| -> bool { t.grid.borrow().neighbour(t.id, dir).is_some() })
		.method("getNextTile", |t: &mut TileHandle, vm: &SQVM, dir: i64| -> Result<SqStrongRef, String> {
			let id = t.grid.borrow().neighbour(t.id, dir).ok_or("getNextTile: no tile in that direction")?;
			tile_handle(&t.grid, vm, id).ok_or_else(|| "getNextTile: no tile class".to_string())
		})
		.method("isSameTileAs", |t: &mut TileHandle, other: Native<TileHandle>| -> bool { t.id == other.0.id })
		.method("getDistanceTo", |t: &mut TileHandle, other: Native<TileHandle>| -> i64 {
			let g = t.grid.borrow();
			hex_distance(&g.tiles[t.id], &g.tiles[other.0.id])
		})
		.method("getDirectionTo", |t: &mut TileHandle, other: Native<TileHandle>| -> i64 {
			super::direction_between(&t.grid.borrow(), t.id, other.0.id) as i64
		})
		.method("getDirection8To", |t: &mut TileHandle, other: Native<TileHandle>| -> i64 {
			world_query::direction8(t.pos(), other.0.pos())
		})
		// Zones of control and of occupation (see `zone`): adding/removing applies to the
		// neighbours; the counts and checks read this tile.
		.method("addZoneOfControl", |t: &mut TileHandle, f: i64| t.grid.borrow_mut().apply_zone(t.id, f, false, true))
		.method("removeZoneOfControl", |t: &mut TileHandle, f: i64| t.grid.borrow_mut().apply_zone(t.id, f, false, false))
		.method("getZoneOfControlCount", |t: &mut TileHandle, f: i64| -> i64 { t.with(|d| d.zoc.count(f)) })
		.method("getZoneOfControlCountOtherThan", |t: &mut TileHandle, allied: SqArray| -> i64 {
			let mask = factions(&allied);
			t.with(|d| d.zoc.count_outside(mask))
		})
		.method("hasZoneOfControlOtherThan", |t: &mut TileHandle, allied: SqArray| -> bool {
			let mask = factions(&allied);
			t.with(|d| d.zoc.has_outside(mask))
		})
		.method("addZoneOfOccupation", |t: &mut TileHandle, f: i64| t.grid.borrow_mut().apply_zone(t.id, f, true, true))
		.method("removeZoneOfOccupation", |t: &mut TileHandle, f: i64| t.grid.borrow_mut().apply_zone(t.id, f, true, false))
		.method("getZoneOfOccupationCount", |t: &mut TileHandle, f: i64| -> i64 { t.with(|d| d.zoo.count(f)) })
		.method("getZoneOfOccupationCountOtherThan", |t: &mut TileHandle, allied: SqArray| -> i64 {
			let mask = factions(&allied);
			t.with(|d| d.zoo.count_outside(mask))
		})
		.method("hasZoneOfOccupationOtherThan", |t: &mut TileHandle, allied: SqArray| -> bool {
			let mask = factions(&allied);
			t.with(|d| d.zoo.has_outside(mask))
		});
	b.build(vm)
}

fn xy(vm: &SQVM, x: i64, y: i64) -> SqStrongRef {
	let mut t = SqTable::new(vm);
	t.set("X", x);
	t.set("Y", y);
	t.to_strong()
}
