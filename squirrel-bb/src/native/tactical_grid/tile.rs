//! The `TacticalTile` script class: one battle tile's fields and methods.

use squirrel::{
	ClassBuilder, Native, Number, Retained, SQVM, SqArray, SqClass, SqStrongRef, SqTable, StackArgs, This, Variadic,
};

use super::{Grid, TacticalTileData, detail, hex_distance, hex_distance_to, picking, tile_handle, vec};
use crate::native::overload::{Overload, overload_key};
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
pub(super) fn properties(t: &TileHandle, vm: &SQVM) -> SqStrongRef {
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

/// The set of a script array of faction ids.
fn factions(list: &SqArray) -> super::zone::FactionSet {
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
		$b.property_ro::<$ty>($name, $get)
	};
}

/// `setColor(r, g, b)` with 0–1 channels, or `setColor("#rrggbb")`: the tile's tint
/// (`0x4da490`, `0x4da500`).
fn set_color(t: &mut TileHandle, vm: &SQVM, Variadic(args): Variadic<Retained>) {
	let number = |i: usize| args.get(i).and_then(|a| a.0.get_value::<Number>(vm)).map(|n| n.0 as f32);
	let color = match (number(0), number(1), number(2)) {
		(Some(r), Some(g), Some(b)) => {
			let byte = |v: f32| ((v * 255.0) as i32 as u32) & 0xff;
			Some(0xff00_0000 | byte(r) << 16 | byte(g) << 8 | byte(b))
		}
		_ => {
			let text = args.first().and_then(|a| a.0.get_value::<String>(vm));
			text.and_then(|s| crate::native::color::parse_color(&s))
		}
	};
	if let Some(color) = color {
		t.with_mut(|d| d.color = color);
	}
}

/// `clear()`: every detail; `clear(flags)`: those carrying any of `flags`.
fn clear(t: &mut TileHandle, vm: &SQVM, Variadic(args): Variadic<Retained>) {
	let flags = args.first().and_then(|a| a.0.get_value::<Number>(vm)).map(|n| n.0 as i64);
	clear_tile(t, vm, flags);
}

/// Clear tile `t` of what carries any of `flags` (everything for `None`): its details,
/// and through the clear hook the battle effects lying there (pools, landed heads).
pub(super) fn clear_tile(t: &TileHandle, vm: &SQVM, flags: Option<i64>) {
	t.with_mut(|d| match flags {
		None => d.details.clear(),
		Some(f) => d.details.retain(|detail| detail.borrow().flags & f == 0),
	});
	let hook = t.grid.borrow().clear_hook.clone();
	if let Some(hook) = hook {
		hook(vm, t.id, flags);
	}
}

/// `spawnDetail(brush [, flags [, mirrored [, fadeIn [, offset]]]])`: see `detail`.
fn spawn_detail(t: &mut TileHandle, vm: &SQVM, Variadic(args): Variadic<Retained>) -> Option<SqStrongRef> {
	let brush = args.first()?.0.get_value::<String>(vm)?;
	let number = |i: usize| args.get(i).and_then(|a| a.0.get_value::<Number>(vm)).map(|n| n.0);
	let flag = |i: usize| args.get(i).and_then(|a| a.0.get_value::<bool>(vm));
	let offset = args.get(4).map_or((0.0, 0.0), |a| {
		let c = |axis: &str| a.0.get_member::<Number>(vm, axis).map_or(0.0, |n| n.0 as f32);
		(c("X"), c("Y"))
	});
	let spawn = detail::Spawn {
		brush,
		flags: number(1).map_or(0, |n| n as i64),
		mirrored: if args.len() >= 3 { Some(flag(2).unwrap_or(false)) } else { None },
		fade_in: flag(3).unwrap_or(false),
		offset,
	};
	let now = crate::native::sprite_color::real_time(vm);
	let now_ms = crate::native::sprite_anim::virtual_ms(vm);
	let (class, sprites, rng) = {
		let g = t.grid.borrow();
		(g.detail_class.clone()?, g.sprites.clone(), g.engine_rng.clone())
	};
	let detail = t.with_mut(|d| detail::spawn(&mut d.details, spawn, &sprites, &rng, now, now_ms))?;
	Some(class.instantiate(vm, detail::DetailHandle(detail)))
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
	b = ro!(b, "IsDiscovered", |t: &TileHandle| t.with(|d| d.is_discovered), bool);
	b = rw!(b, "IsContainingItemsFlipped", is_containing_items_flipped, bool);
	b = ro!(b, "ID", |t: &TileHandle| t.id as i64, i64);
	// The axial coords (`Coords.X`/`Coords.Y`) as plain fields.
	b = ro!(b, "X", |t: &TileHandle| t.with(|d| d.q), i64);
	b = ro!(b, "Y", |t: &TileHandle| t.with(|d| d.r), i64);
	b = ro!(b, "TVHeight", |t: &TileHandle| t.with(|d| d.tv_height), i64);
	b = ro!(b, "TVTerrain", |t: &TileHandle| t.with(|d| d.tv_terrain), i64);
	b = ro!(b, "TVLevelDisadvantage", |t: &TileHandle| t.with(|d| d.tv_level_disadvantage), i64);
	b = ro!(b, "TVTotal", |t: &TileHandle| t.with(|d| d.tv_total), i64);
	b = ro!(b, "IsVisibleForPlayer", |t: &TileHandle| t.with(|d| d.vis_mask.contains(1)), bool);
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
		// setColor, clear and spawnDetail are Sqrat overloads (`__sqrat_ol_ <name>_<N>`
		// leaves under a strict dispatcher, see `overload`); each leaf reads its
		// arguments as below.
		.method(&overload_key("setColor", 1), set_color)
		.method(&overload_key("setColor", 3), set_color)
		.native_method("setColor", {
			let overload = Overload::new("setColor");
			move |vm: &SQVM, this: This, args: StackArgs| overload.dispatch(vm, this, args)
		})
		.method(&overload_key("clear", 0), clear)
		.method(&overload_key("clear", 1), clear)
		.native_method("clear", {
			let overload = Overload::new("clear");
			move |vm: &SQVM, this: This, args: StackArgs| overload.dispatch(vm, this, args)
		});
	for arity in 1..=5 {
		b = b.method(&overload_key("spawnDetail", arity), spawn_detail);
	}
	b = b
		.native_method("spawnDetail", {
			let overload = Overload::new("spawnDetail");
			move |vm: &SQVM, this: This, args: StackArgs| overload.dispatch(vm, this, args)
		})
		.method("hasNextTile", |t: &mut TileHandle, dir: i64| -> bool { t.grid.borrow().neighbour(t.id, dir).is_some() })
		.method("getNextTile", |t: &mut TileHandle, vm: &SQVM, dir: i64| -> Result<SqStrongRef, String> {
			let id = t.grid.borrow().neighbour(t.id, dir).ok_or("getNextTile: no tile in that direction")?;
			tile_handle(&t.grid, vm, id).ok_or_else(|| "getNextTile: no tile class".to_string())
		})
		.method("isSameTileAs", |t: &mut TileHandle, other: Native<TileHandle>| -> bool { t.id == other.0.id })
		.method("getPosBetweenThisAnd", |t: &mut TileHandle, vm: &SQVM, other: Native<TileHandle>| -> Result<Option<SqStrongRef>, String> {
			let (x, y) = picking::midpoint(&t.grid.borrow(), t.id, other.0.id);
			vec(vm, (x as f64, y as f64))
		})
		.method("getTileBetweenThisAnd", |t: &mut TileHandle, vm: &SQVM, other: Native<TileHandle>| -> Result<SqStrongRef, String> {
			let id = if t.id == other.0.id {
				t.id
			} else {
				let mut g = t.grid.borrow_mut();
				let masks = g.hex_masks().ok_or("getTileBetweenThisAnd: the game data has no gfx/masks/hex_mask.0-3")?;
				let level = g.camera.level;
				picking::tile_between(&g, &masks, level, t.id, other.0.id)
			};
			tile_handle(&t.grid, vm, id).ok_or_else(|| "getTileBetweenThisAnd: no tile class".to_string())
		})
		// getDistanceTo(tile) or (x, y) in axial coords: Sqrat overloads.
		.method(&overload_key("getDistanceTo", 1), |t: &mut TileHandle, other: Native<TileHandle>| -> i64 {
			let g = t.grid.borrow();
			hex_distance(&g.tiles[t.id], &g.tiles[other.0.id])
		})
		.method(&overload_key("getDistanceTo", 2), |t: &mut TileHandle, x: i64, y: i64| -> i64 {
			hex_distance_to(&t.grid.borrow().tiles[t.id], x, y)
		})
		.native_method("getDistanceTo", {
			let overload = Overload::new("getDistanceTo");
			move |vm: &SQVM, this: This, args: StackArgs| overload.dispatch(vm, this, args)
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
			t.with(|d| d.zoc.count_outside(&mask))
		})
		.method("hasZoneOfControlOtherThan", |t: &mut TileHandle, allied: SqArray| -> bool {
			let mask = factions(&allied);
			t.with(|d| d.zoc.has_outside(&mask))
		})
		.method("addZoneOfOccupation", |t: &mut TileHandle, f: i64| t.grid.borrow_mut().apply_zone(t.id, f, true, true))
		.method("removeZoneOfOccupation", |t: &mut TileHandle, f: i64| t.grid.borrow_mut().apply_zone(t.id, f, true, false))
		.method("getZoneOfOccupationCount", |t: &mut TileHandle, f: i64| -> i64 { t.with(|d| d.zoo.count(f)) })
		.method("getZoneOfOccupationCountOtherThan", |t: &mut TileHandle, allied: SqArray| -> i64 {
			let mask = factions(&allied);
			t.with(|d| d.zoo.count_outside(&mask))
		})
		.method("hasZoneOfOccupationOtherThan", |t: &mut TileHandle, allied: SqArray| -> bool {
			let mask = factions(&allied);
			t.with(|d| d.zoo.has_outside(&mask))
		});
	b.build(vm)
}

fn xy(vm: &SQVM, x: i64, y: i64) -> SqStrongRef {
	let mut t = SqTable::new(vm);
	t.set("X", x);
	t.set("Y", y);
	t.to_strong()
}
