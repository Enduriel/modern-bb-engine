//! The navigator's path settings (`navigator.createSettings()`), laid out like the
//! original's (offsets from its binding getters/setters; defaults from its reset).
//!
//! There is one settings object per navigator: `createSettings` resets it to the
//! defaults and returns it, `getLastSettings` returns it as is, so every handle a script
//! holds aliases the same values, as in the original.

use std::cell::RefCell;
use std::rc::Rc;

use squirrel::{ClassBuilder, Number, SQVM, SqArray, SqClass, SqStrongRef};

use crate::native::tactical_grid::{self, Grid, zone::{self, FactionSet}};

#[derive(Clone)]
pub(crate) struct NavSettings {
	/// `ActionPointCosts` / `FatigueCosts`: per terrain type.
	pub ap_costs: [i32; 16],
	pub fatigue_costs: [i32; 16],
	/// `FatigueCostFactor`: how much fatigue weighs in a step's search cost next to its
	/// AP (an f32, 0 after a reset).
	pub fatigue_weight: f32,
	pub ap_per_level: i32,
	pub fatigue_per_level: i32,
	pub max_level_difference: i32,
	pub allow_zoc_passing: bool,
	pub is_player: bool,
	pub zoc_cost: i32,
	pub height_level_cost: i32,
	pub hidden_cost: i32,
	pub avoid_cost: i32,
	pub avoid_max_distance: i32,
	pub heat_cost: i32,
	/// `Faction`: stored, not read by the search.
	pub faction: i32,
	/// `IgnoreEntitiesOfFaction` (the original's `+0xc0`, 0 by default): stored; the
	/// search doesn't apply it yet (vanilla never sets it).
	pub ignore_faction: i32,
	/// `AlliedFactions` once assigned: every other faction is hostile. `None` after a
	/// reset: none is. (The original keeps the hostile ones as a 64-bit mask, so a
	/// faction id of 64 or more is never hostile there; see
	/// `docs/differences.md`.)
	pub allied: Option<FactionSet>,
	pub consider_empty: Option<usize>,
	pub avoid_tile: Option<usize>,
}

impl Default for NavSettings {
	fn default() -> Self {
		NavSettings {
			ap_costs: [2; 16],
			fatigue_costs: [0; 16],
			fatigue_weight: 0.0,
			ap_per_level: 0,
			fatigue_per_level: 0,
			max_level_difference: 1,
			allow_zoc_passing: true,
			is_player: false,
			zoc_cost: 0,
			height_level_cost: 0,
			hidden_cost: 0,
			avoid_cost: 0,
			avoid_max_distance: 0,
			heat_cost: 0,
			faction: 0,
			ignore_faction: 0,
			allied: None,
			consider_empty: None,
			avoid_tile: None,
		}
	}
}

impl NavSettings {
	/// Whether faction `f` is hostile.
	pub fn is_hostile(&self, f: i64) -> bool {
		self.allied.as_ref().is_some_and(|a| !a.contains(f))
	}

	/// Whether a hostile faction exerts zone `z`.
	pub fn hostile_zone(&self, z: &zone::Zone) -> bool {
		self.allied.as_ref().is_some_and(|a| z.has_outside(a))
	}

	/// The summed counts of zone `z` from hostile factions.
	pub fn hostile_zone_count(&self, z: &zone::Zone) -> i64 {
		self.allied.as_ref().map_or(0, |a| z.count_outside(a))
	}
}

/// A script handle to the navigator's settings.
#[derive(Clone)]
pub(crate) struct SettingsHandle(pub Rc<RefCell<NavSettings>>);

impl SettingsHandle {
	fn with<R>(&self, f: impl FnOnce(&NavSettings) -> R) -> R {
		f(&self.0.borrow())
	}
	fn with_mut<R>(&self, f: impl FnOnce(&mut NavSettings) -> R) -> R {
		f(&mut self.0.borrow_mut())
	}
}

/// Read a cost array into `dst` (as many entries as given, up to 16; numbers truncate).
fn read_costs(vm: &SQVM, value: &SqStrongRef, dst: &mut [i32; 16]) -> Result<(), String> {
	if is_null(vm, value) {
		return Ok(());
	}
	let array = value.get_value::<SqArray>(vm).ok_or("expected an array of costs")?;
	for (slot, n) in dst.iter_mut().zip(array.to_vec::<Number>()) {
		*slot = n.0 as i32;
	}
	Ok(())
}

/// Whether `value` is `null`.
fn is_null(vm: &SQVM, value: &SqStrongRef) -> bool {
	value.get_value::<Option<i64>>(vm).is_some_and(|v| v.is_none())
}

/// The tile a setting refers to (`null` clears it).
fn read_tile(vm: &SQVM, grid: &Grid, value: &SqStrongRef) -> Result<Option<usize>, String> {
	if is_null(vm, value) {
		return Ok(None);
	}
	tactical_grid::tile_id_of(grid, vm, value).map(Some).ok_or_else(|| "expected a tile".to_string())
}

macro_rules! int_field {
	($b:expr, $name:literal, $field:ident) => {
		$b.property(
			$name,
			|s: &SettingsHandle| s.with(|d| d.$field as i64),
			|s: &mut SettingsHandle, v: i64| s.with_mut(|d| d.$field = v as i32),
		)
	};
}

macro_rules! bool_field {
	($b:expr, $name:literal, $field:ident) => {
		$b.property(
			$name,
			|s: &SettingsHandle| s.with(|d| d.$field),
			|s: &mut SettingsHandle, v: bool| s.with_mut(|d| d.$field = v),
		)
	};
}

pub(super) fn build_class(vm: &mut SQVM, grid: Grid) -> SqClass<SettingsHandle> {
	let mut b = ClassBuilder::<SettingsHandle>::new("TacticalNavigatorSettings");
	b = int_field!(b, "ActionPointCostPerLevel", ap_per_level);
	b = int_field!(b, "FatigueCostPerLevel", fatigue_per_level);
	b = int_field!(b, "MaxLevelDifference", max_level_difference);
	b = int_field!(b, "ZoneOfControlCost", zoc_cost);
	b = int_field!(b, "HeightLevelCost", height_level_cost);
	b = int_field!(b, "HiddenCost", hidden_cost);
	b = int_field!(b, "AvoidCost", avoid_cost);
	b = int_field!(b, "AvoidMaxDistance", avoid_max_distance);
	b = int_field!(b, "HeatCost", heat_cost);
	b = int_field!(b, "Faction", faction);
	b = int_field!(b, "IgnoreEntitiesOfFaction", ignore_faction);
	b = b.property(
		"FatigueCostFactor",
		|s: &SettingsHandle| s.with(|d| d.fatigue_weight as f64),
		|s: &mut SettingsHandle, v: f64| s.with_mut(|d| d.fatigue_weight = v as f32),
	);
	b = bool_field!(b, "AllowZoneOfControlPassing", allow_zoc_passing);
	b = bool_field!(b, "IsPlayer", is_player);
	// They read back null in the original, even once set, and null is taken without an
	// error (probed): here it changes nothing.
	b = b
		.property_obj_rw(
			"ActionPointCosts",
			|_s, _vm| None,
			|s, vm, v| s.with_mut(|d| read_costs(vm, &v, &mut d.ap_costs)),
		)
		.property_obj_rw(
			"FatigueCosts",
			|_s, _vm| None,
			|s, vm, v| s.with_mut(|d| read_costs(vm, &v, &mut d.fatigue_costs)),
		)
		.property_obj_rw(
			"AlliedFactions",
			|_s, _vm| None,
			|s, vm, v| {
				if is_null(vm, &v) {
					return Ok(());
				}
				let array = v.get_value::<SqArray>(vm).ok_or("AlliedFactions: expected an array")?;
				let allied: Vec<i64> = array.to_vec::<Number>().into_iter().map(|n| n.0 as i64).collect();
				s.with_mut(|d| d.allied = Some(zone::faction_mask(&allied)));
				Ok(())
			},
		);
	let g = grid.clone();
	b = b.property_obj_rw(
		"TileToConsiderEmpty",
		|_s, _vm| None,
		move |s, vm, v| {
			let tile = read_tile(vm, &g, &v)?;
			s.with_mut(|d| d.consider_empty = tile);
			Ok(())
		},
	);
	let g = grid;
	b = b.property_obj_rw(
		"AvoidTile",
		|_s, _vm| None,
		move |s, vm, v| {
			let tile = read_tile(vm, &g, &v)?;
			s.with_mut(|d| d.avoid_tile = tile);
			Ok(())
		},
	);
	b.build(vm)
}
