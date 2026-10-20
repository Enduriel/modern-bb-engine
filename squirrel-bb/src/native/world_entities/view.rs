//! What the player can see, and the entity natives built on positions: visibility,
//! discovery, distances and directions. All follow the original's disassembly
//! (2026-10-13).
//!
//! The player's view is two values the player party pushes every update
//! (`World.setPlayerPos` and `World.setPlayerVisionRadius`). An entity is hidden from
//! the player when it is farther away than its own `Visibility` (default 1.0) times
//! that radius. The player's own faction is never hidden.

use std::cell::RefCell;
use std::rc::Rc;

use squirrel::{Number, Retained, SQVM, SqStrongRef, SqTable};

use super::{EntityMachine, ignore_missing, vec_xy};
use crate::native::overload::single_overload;
use crate::native::world_query;

/// `Const.Faction.Player`.
const PLAYER_FACTION: i64 = 1;

/// The player's view, as last pushed by the player party.
#[derive(Clone, Copy)]
pub(super) struct PlayerView {
	pub pos: (f64, f64),
	pub vision_radius: f32,
}

impl Default for PlayerView {
	fn default() -> Self {
		// The original world starts with the view at the origin and radius 1.0.
		PlayerView {
			pos: (0.0, 0.0),
			vision_radius: 1.0,
		}
	}
}

type Machine = Rc<RefCell<EntityMachine>>;

/// `isHiddenToPlayer`: farther from the player's view than the entity's own visibility
/// times the view's radius. The player's own faction is never hidden.
pub(super) fn hidden_to_player(rec: &super::EntityRecord, view: &PlayerView) -> bool {
	rec.faction != PLAYER_FACTION && world_query::distance(rec.pos, view.pos) > rec.visibility * view.vision_radius
}

/// The position of the entity an argument refers to, if it is a registered entity.
fn entity_pos(m: &Machine, other: &SqStrongRef) -> Option<(f64, f64)> {
	m.borrow().record(other.identity()).map(|rec| rec.pos)
}

/// The calling entity's position (origin if unregistered).
fn this_pos(m: &Machine, vm: &SQVM) -> (f64, f64) {
	m.borrow().record(vm.this_identity()).map_or((0.0, 0.0), |rec| rec.pos)
}

/// A boolean flag on the calling entity: `(getter, setter)` closures.
fn flag(
	m: &Machine,
	vm: &mut SQVM,
	get: fn(&super::EntityRecord) -> bool,
	set: fn(&mut super::EntityRecord, bool),
) -> (SqStrongRef, SqStrongRef) {
	let getter = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM| -> bool { m.borrow().record(vm.this_identity()).is_some_and(get) })
	};
	let setter = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM, v: bool| {
			if let Some(rec) = m.borrow_mut().record_mut(vm.this_identity()) {
				set(rec, v);
			}
		})
	};
	(getter, setter)
}

/// The view, visibility and geometry natives injected onto every world entity.
pub(super) fn entity_methods(m: &Machine, vm: &mut SQVM) -> Vec<(&'static str, SqStrongRef)> {
	let mut out: Vec<(&'static str, SqStrongRef)> = Vec::new();

	let is_hidden_to_player = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM| -> bool {
			let machine = m.borrow();
			machine
				.record(vm.this_identity())
				.is_some_and(|rec| hidden_to_player(rec, &machine.player_view))
		})
	};
	// isVisibleToEntity(other, radius): within this entity's Visibility * radius.
	let is_visible_to_entity = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM, other: Retained, radius: Number| -> bool {
			let Some(other_pos) = entity_pos(&m, &other.0) else {
				return false;
			};
			let machine = m.borrow();
			let Some(rec) = machine.record(vm.this_identity()) else {
				return false;
			};
			world_query::distance(rec.pos, other_pos) <= rec.visibility * radius.0 as f32
		})
	};
	let get_distance_to = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM, other: Retained| -> f64 {
			entity_pos(&m, &other.0).map_or(0.0, |p| f64::from(world_query::distance(p, this_pos(&m, vm))))
		})
	};
	let get_direction_to = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM, other: Retained| -> i64 {
			entity_pos(&m, &other.0).map_or(0, |p| world_query::direction6(this_pos(&m, vm), p))
		})
	};
	let get_direction_to_pos = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM, pos: Retained| -> i64 {
			world_query::direction6(this_pos(&m, vm), vec_xy(vm, &pos.0))
		})
	};
	let get_direction8_to = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM, pos: Retained| -> i64 {
			world_query::direction8(this_pos(&m, vm), vec_xy(vm, &pos.0))
		})
	};
	let get_visibility = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM| -> f64 {
			m.borrow().record(vm.this_identity()).map_or(1.0, |rec| f64::from(rec.visibility))
		})
	};
	let set_visibility = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM, v: Number| {
			if let Some(rec) = m.borrow_mut().record_mut(vm.this_identity()) {
				rec.visibility = v.0 as f32;
			}
		})
	};
	// setDiscovered(b): the first discovery fires `onDiscovered`; every call then fires
	// `onVisibleToPlayer` (when the script defines it), as in the original.
	let set_discovered = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM, discovered: bool| -> Result<(), String> {
			let (entity, first) = {
				let mut machine = m.borrow_mut();
				let Some(rec) = machine.record_mut(vm.this_identity()) else {
					return Ok(());
				};
				let first = !rec.discovered && discovered;
				rec.discovered = discovered;
				// A dead entity's record has let go of it; `this` is it (bound to the leaf).
				(rec.entity.clone().unwrap_or_else(|| vm.this_ref()), first)
			};
			if first {
				ignore_missing(entity.call_method(vm, "onDiscovered", ()))?;
			}
			ignore_missing(entity.call_method(vm, "onVisibleToPlayer", ()))
		})
	};
	let get_z_offset = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM| -> i64 { m.borrow().record(vm.this_identity()).map_or(0, |rec| rec.z_offset) })
	};
	let set_z_offset = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM, z: i64| {
			if let Some(rec) = m.borrow_mut().record_mut(vm.this_identity()) {
				rec.z_offset = z;
			}
		})
	};

	let (is_visible, _) = flag(m, vm, |r| r.visible, |r, v| r.visible = v);
	// Shown again, its sprites' fades go on (see `moving_sprites`).
	let set_visible = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM, v: bool| {
			let mut machine = m.borrow_mut();
			let Some(&idx) = machine.by_identity.get(&vm.this_identity()) else { return };
			machine.entities[idx].visible = v;
			if v {
				let owner = machine.owner(idx);
				machine.moving.note(owner);
			}
		})
	};
	let (is_discovered, _) = flag(m, vm, |r| r.discovered, |r, v| r.discovered = v);
	let (is_visible_in_fow, set_visible_in_fow) =
		flag(m, vm, |r| r.visible_in_fog_of_war, |r, v| r.visible_in_fog_of_war = v);
	let (is_ignoring_collision, set_ignore_collision) =
		flag(m, vm, |r| r.ignore_collision, |r, v| r.ignore_collision = v);
	let (is_important, set_important) = flag(m, vm, |r| r.important, |r, v| r.important = v);
	let (is_rendered_top, set_rendered_top) = flag(m, vm, |r| r.rendered_top, |r, v| r.rendered_top = v);

	out.extend([
		("isHiddenToPlayer", is_hidden_to_player),
		("isVisibleToEntity", is_visible_to_entity),
		("getDistanceTo", get_distance_to),
		("getDirectionTo", get_direction_to),
		("getDirectionToPos", get_direction_to_pos),
		("getDirection8To", get_direction8_to),
		("getVisibility", get_visibility),
		("setVisibility", set_visibility),
		("isDiscovered", is_discovered),
		("getZOffset", get_z_offset),
		("setZOffset", set_z_offset),
		("isVisible", is_visible),
		("setVisible", set_visible),
		("isVisibleInFogOfWar", is_visible_in_fow),
		("setVisibleInFogOfWar", set_visible_in_fow),
		("isIgnoringCollision", is_ignoring_collision),
		("setIgnoreCollision", set_ignore_collision),
		("isImportant", is_important),
		("setImportant", set_important),
		("isRenderedTop", is_rendered_top),
		("setRenderedTop", set_rendered_top),
	]);
	// The original binds it as a one-arity Sqrat overload.
	out.extend(single_overload(vm, "setDiscovered", "__sqrat_ol_ setDiscovered_1", set_discovered));
	out
}

/// `World.setPlayerPos`/`getPlayerPos`, `World.setPlayerVisionRadius` and
/// `World.getPlayerVisionRadius`.
pub(super) fn world_methods(m: &Machine, vm: &mut SQVM) -> Vec<(String, SqStrongRef)> {
	let set_player_pos = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM, pos: Retained| {
			let pos = vec_xy(vm, &pos.0);
			// Which footprints show follows it (see `world_scene_changes`).
			if std::mem::replace(&mut m.borrow_mut().player_view.pos, pos) != pos {
				crate::native::world_scene_changes::note();
			}
		})
	};
	// getPlayerPos(): what setPlayerPos stored, an f32 vector (disassembled: World+0x120).
	let get_player_pos = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM| -> Option<SqStrongRef> {
			let (x, y) = m.borrow().player_view.pos;
			SqTable::root(vm)
				.get::<SqStrongRef>("createVec")
				.and_then(|create| create.call_ret::<SqStrongRef, _>(vm, (x as f32 as f64, y as f32 as f64)).ok().flatten())
		})
	};
	let set_player_vision_radius = {
		let m = m.clone();
		vm.make_closure(move |r: Number| {
			let r = r.0 as f32;
			if std::mem::replace(&mut m.borrow_mut().player_view.vision_radius, r) != r {
				crate::native::world_scene_changes::note();
			}
		})
	};
	let get_player_vision_radius = {
		let m = m.clone();
		vm.make_closure(move || f64::from(m.borrow().player_view.vision_radius))
	};
	vec![
		("setPlayerPos".to_string(), set_player_pos),
		("getPlayerPos".to_string(), get_player_pos),
		("setPlayerVisionRadius".to_string(), set_player_vision_radius),
		("getPlayerVisionRadius".to_string(), get_player_vision_radius),
	]
}
