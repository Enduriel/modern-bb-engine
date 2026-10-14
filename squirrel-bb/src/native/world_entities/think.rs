//! World-entity thinkers: `registerThinker` / `unregisterThinker`, and the per-frame
//! think pass that calls each registered entity's script `onUpdate`. Follows the
//! original's disassembly (2026-10-13).
//!
//! The pass runs from `State.updateScene()` (the world scene's update), and only
//! while the world is shown (not during a battle; see [`super::scene_shown`]) and its
//! speed is non-zero. It visits thinkers in registration order,
//! including any registered during the pass. Each one:
//!
//! * is dropped if it is no longer in the scene (it died);
//! * if not `Important` and more than 2000 px from the player's view, is `OutOfRange`
//!   and only updated once its own interval (0.5–1.5 s of virtual time) has passed
//!   since its last update;
//! * otherwise gets its `onUpdate`, and stays registered while it is alive.
//!
//! `unregisterThinker` during the pass does nothing, as in the original (an entity
//! leaves the list by dying).
//!
//! Divergence: the original draws each entity's interval from an engine RNG separate
//! from the script one. We derive it from the entity id, so runs are deterministic.

use std::cell::RefCell;
use std::rc::Rc;

use squirrel::{Number, SQVM, SqStrongRef, SqTable};

use super::{EntityMachine, ignore_missing};
use crate::native::world_query;

/// Beyond this distance from the player's view (px), a non-important entity is
/// `OutOfRange`.
const NEAR_RANGE: f32 = 2000.0;

/// Thinker bookkeeping for one entity.
#[derive(Clone, Copy, Default)]
pub(super) struct ThinkState {
	/// Virtual time (s) of the entity's last `onUpdate`.
	last_update: f32,
	/// `isOutOfRange()`: whether the last pass found it far away.
	pub out_of_range: bool,
}

/// The update interval for an out-of-range entity: 0.5–1.5 s, spread by id.
fn interval(id: i64) -> f32 {
	let spread = (id as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 40;
	0.5 + spread as f32 / (1u64 << 24) as f32
}

/// The thinker list and whether a pass is running.
#[derive(Default)]
pub(super) struct Thinkers {
	/// Record indices, in registration order (duplicates allowed, as in the original).
	list: Vec<usize>,
	in_pass: bool,
}

impl Thinkers {
	/// Registered thinkers, counting duplicates.
	pub(super) fn len(&self) -> usize {
		self.list.len()
	}
}

type Machine = Rc<RefCell<EntityMachine>>;

/// `registerThinker` and `unregisterThinker`, for injection onto every entity.
pub(super) fn entity_methods(m: &Machine, vm: &mut SQVM) -> Vec<(&'static str, SqStrongRef)> {
	let register = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM| {
			let mut machine = m.borrow_mut();
			if let Some(&idx) = machine.by_identity.get(&vm.this_identity()) {
				machine.thinkers.list.push(idx);
			}
		})
	};
	let unregister = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM| {
			let mut machine = m.borrow_mut();
			if machine.thinkers.in_pass {
				return;
			}
			if let Some(&idx) = machine.by_identity.get(&vm.this_identity()) {
				machine.thinkers.list.retain(|&i| i != idx);
			}
		})
	};
	let is_out_of_range = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM| -> bool {
			m.borrow().record(vm.this_identity()).is_some_and(|rec| rec.think.out_of_range)
		})
	};
	vec![
		("registerThinker", register),
		("unregisterThinker", unregister),
		("isOutOfRange", is_out_of_range),
	]
}

/// Read a number from a script-side getter, e.g. `Time.getVirtualTimeF()`.
fn script_number(vm: &SQVM, table: &str, getter: &str) -> Result<f64, String> {
	let t = SqTable::root(vm).get::<SqStrongRef>(table).ok_or_else(|| format!("{table} missing"))?;
	Ok(t.call_method_ret::<Number, _>(vm, getter, ())?.map_or(0.0, |n| n.0))
}

/// One think pass (see the module docs). No machine borrow is held while scripts run.
pub(super) fn run_pass(m: &Machine, vm: &SQVM) -> Result<(), String> {
	if !super::scene_shown(vm)? || script_number(vm, "World", "getSpeedMult")? == 0.0 {
		return Ok(());
	}
	let now = script_number(vm, "Time", "getVirtualTimeF")? as f32;
	let real_now = script_number(vm, "Time", "getRealTime")? as i64;
	m.borrow_mut().thinkers.in_pass = true;
	let result = think_all(m, vm, now, real_now);
	m.borrow_mut().thinkers.in_pass = false;
	result
}

fn think_all(m: &Machine, vm: &SQVM, now: f32, real_now: i64) -> Result<(), String> {
	let mut i = 0;
	loop {
		// A faded-out entity dies here (dropped from the list below).
		let pending = m.borrow().thinkers.list.get(i).copied();
		if let Some(idx) = pending {
			super::reap(m, vm, idx, real_now)?;
		}
		// Decide under the borrow; run the script without it.
		let call = {
			let mut machine = m.borrow_mut();
			let Some(&idx) = machine.thinkers.list.get(i) else {
				return Ok(());
			};
			if !machine.entities[idx].alive {
				machine.thinkers.list.remove(i);
				continue;
			}
			let view_pos = machine.player_view.pos;
			let rec = &mut machine.entities[idx];
			let far = !rec.important && world_query::distance(rec.pos, view_pos) > NEAR_RANGE;
			if far && now - rec.think.last_update < interval(rec.id) {
				i += 1;
				continue;
			}
			rec.think.out_of_range = far;
			rec.think.last_update = now;
			(idx, rec.entity.clone())
		};
		let (idx, entity) = call;
		ignore_missing(entity.call_method(vm, "onUpdate", ()))?;
		if m.borrow().entities[idx].alive {
			i += 1;
		} else {
			m.borrow_mut().thinkers.list.remove(i);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::interval;

	#[test]
	fn intervals_span_half_to_one_and_a_half_seconds() {
		let all: Vec<f32> = (0..1000).map(interval).collect();
		assert!(all.iter().all(|&s| (0.5..1.5).contains(&s)));
		let mean = all.iter().sum::<f32>() / all.len() as f32;
		assert!((mean - 1.0).abs() < 0.05, "mean {mean}");
	}
}
