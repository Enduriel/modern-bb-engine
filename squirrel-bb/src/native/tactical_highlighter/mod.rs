//! The battle's highlighter, `Tactical.getHighlighter()` (`Tactical+0x1c0`;
//! disassembled, registration `0x4b5400`): the tiles a skill reaches and
//! the ones it can target, the tiles where hostile zones of control stop a move, and the
//! icons over the tiles an attack would hit.
//!
//! Each tile keeps one highlight (see [`Highlight`]); a later one replaces an earlier
//! one, and they stay until `clear()` or `clearHighlights()`. The scripts clear before
//! every highlight.
//!
//! - `highlightZoneOfControl(alliedFactions)` (`0x4b49a0`, see `zones`);
//! - `highlightRangeOfSkill(skill, actor)` (`0x4b4c20`, see `range`);
//! - `addOverlayIcon(brush, tile, x, y)` and `clearOverlayIcons()` (`0x4b5200`,
//!   `0x4b5350`, see `icons`);
//! - `clearHighlights()` (`0x4b5140`) and `clear()` (`0x4b53e0`, both).
//!
//! It also keeps the blocked-tile highlights the `Tactical` table makes (see `blocked`).

use std::cell::RefCell;
use std::rc::Rc;

use squirrel::{ClassBuilder, Number, Retained, SQVM, SqClass, SqStrongRef, SqTable};

use crate::native::tactical_entities::{Units, unit_identity};
use crate::native::tactical_grid::{Grid, TacticalGrid};
use crate::scene::tactical::SceneOverlayIcon;

mod blocked;
mod icons;
mod range;
mod zones;

/// A tile's highlight (tile `+0x14`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Highlight {
	#[default]
	None,
	/// In a hostile zone of control, next to the enemy that exerts it.
	ZoneOfControl,
	/// In a skill's range.
	InRange,
	/// A tile the skill can be used on.
	Target,
}

impl Highlight {
	/// The brush and colour (ARGB) the battle's tile layer draws it with, over the
	/// tile's terrain (`0x55ac88`; the range's alpha is the tile layer's flag `4`).
	pub(crate) fn look(self) -> Option<(&'static str, u32)> {
		match self {
			Highlight::None => None,
			Highlight::ZoneOfControl => Some(("zone_of_control_overlay", 0xffff_ffff)),
			Highlight::InRange => Some(("zone_range_overlay", 0xc0ff_ffff)),
			Highlight::Target => Some(("zone_target_overlay", 0xffff_ffff)),
		}
	}
}

struct State {
	grid: Grid,
	units: Units,
	/// Some tile has a highlight (`clearHighlights` only walks the map then).
	dirty: bool,
	icons: icons::Icons,
	blocked: blocked::Blocked,
	/// The one `Tactical.getHighlighter()` instance.
	instance: Option<SqStrongRef>,
}

#[derive(Clone)]
struct Handle(Rc<RefCell<State>>);

/// The highlighter service (cheap to clone; clones share it).
#[derive(Clone)]
pub struct TacticalHighlighter(Rc<RefCell<State>>);

impl TacticalHighlighter {
	pub(crate) fn new(grid: &TacticalGrid, units: Units) -> Self {
		TacticalHighlighter(Rc::new(RefCell::new(State {
			grid: grid.handle(),
			units,
			dirty: false,
			icons: icons::Icons::default(),
			blocked: blocked::Blocked::default(),
			instance: None,
		})))
	}

	/// Set `Tactical.getHighlighter()` and the blocked-tile highlights' natives, and end
	/// the icons and marks in `Tactical.clearScene()`. Call after the battle map is
	/// installed (its `clearScene` is wrapped).
	pub fn install(&self, vm: &mut SQVM) {
		let class = build_class(vm);
		let instance = class.instantiate(vm, Handle(self.0.clone()));
		self.0.borrow_mut().instance = Some(instance.clone());
		let get = vm.make_closure(move || instance.clone());
		let inner_clear = SqTable::root(vm).get::<SqTable>("Tactical").and_then(|t| t.get::<SqStrongRef>("clearScene"));
		let clear_scene = {
			let st = self.0.clone();
			vm.make_closure(move |vm: &SQVM| -> Result<(), String> {
				{
					let mut st = st.borrow_mut();
					st.icons = icons::Icons::default();
					st.blocked = blocked::Blocked::default();
					st.dirty = false;
				}
				match &inner_clear {
					Some(inner) => inner.call(vm, ()),
					None => Ok(()),
				}
			})
		};
		let create_blocked = {
			let st = self.0.clone();
			vm.make_closure(move |vm: &SQVM| {
				let shown = crate::native::temp_gameplay_flag(vm, "HighlightBlockedTiles");
				let mut st = st.borrow_mut();
				let grid = st.grid.clone();
				st.blocked.create(&grid.borrow(), shown);
			})
		};
		let show_blocked = {
			let st = self.0.clone();
			vm.make_closure(move |on: bool| st.borrow_mut().blocked.show(on))
		};
		let clear_blocked = {
			let st = self.0.clone();
			vm.make_closure(move || st.borrow_mut().blocked.clear())
		};
		if let Some(mut t) = SqTable::root(vm).get::<SqTable>("Tactical") {
			t.set("getHighlighter", get);
			t.set("clearScene", clear_scene);
			t.set("createBlockedTileHighlights", create_blocked);
			t.set("setBlockedTileHighlightsVisibility", show_blocked);
			t.set("clearBlockedTileHighlights", clear_blocked);
		}
	}

	/// The overlay icons as drawn at real time `now` (seconds); faded-out ones go.
	pub(crate) fn scene_icons(&self, now: f32) -> Vec<SceneOverlayIcon> {
		self.0.borrow_mut().icons.scene(now)
	}

	/// The blocked-tile highlights as drawn now (none while hidden).
	pub(crate) fn scene_blocked(&self) -> Vec<SceneOverlayIcon> {
		let st = self.0.borrow();
		let g = st.grid.borrow();
		st.blocked.scene(&g)
	}

	/// Release the script instance. Call before the VM is closed.
	pub fn clear(&self) {
		self.0.borrow_mut().instance = None;
	}
}

/// `clearHighlights()`.
fn clear_highlights(st: &mut State) {
	if st.dirty {
		for t in &mut st.grid.borrow_mut().tiles {
			t.highlight = Highlight::None;
		}
		st.dirty = false;
	}
}

fn build_class(vm: &mut SQVM) -> SqClass<Handle> {
	ClassBuilder::<Handle>::new("TacticalHighlighter")
		.method("highlightZoneOfControl", |h: &mut Handle, vm: &SQVM, allied: Retained| {
			let mut st = h.0.borrow_mut();
			if zones::highlight(&st.grid, &st.units, vm, &allied.0) {
				st.dirty = true;
			}
		})
		.method("highlightRangeOfSkill", |h: &mut Handle, vm: &SQVM, skill: Retained, actor: Retained| -> Result<(), String> {
			let (grid, units) = {
				let st = h.0.borrow();
				(st.grid.clone(), st.units.clone())
			};
			let origin = units.info(unit_identity(vm, &actor.0)).and_then(|u| u.tile);
			if let Some(origin) = origin {
				if range::highlight(&grid, vm, &skill.0, origin)? {
					h.0.borrow_mut().dirty = true;
				}
			}
			Ok(())
		})
		.method("addOverlayIcon", |h: &mut Handle, brush: String, _tile: Retained, x: Number, y: Number| {
			h.0.borrow_mut().icons.add(brush, (x.0 as f32, y.0 as f32));
		})
		.method("clearOverlayIcons", |h: &mut Handle, vm: &SQVM| {
			h.0.borrow_mut().icons.clear(crate::native::sprite_color::real_time(vm));
		})
		.method("clearHighlights", |h: &mut Handle| clear_highlights(&mut h.0.borrow_mut()))
		.method("clear", |h: &mut Handle, vm: &SQVM| {
			let mut st = h.0.borrow_mut();
			clear_highlights(&mut st);
			st.icons.clear(crate::native::sprite_color::real_time(vm));
		})
		.build(vm)
}
