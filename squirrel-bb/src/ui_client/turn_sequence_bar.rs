//! The tactical turn order bar's JS (`turnsequencebar_module.js`), as far as the game's
//! scripts can observe it. Its animations decide when a turn starts: once an entity
//! has slid into the first slot, the JS tells the scripts in three steps,
//! `onEntityEntersFirstSlot` (the turn starts or resumes), `onEntityEnteredFirstSlot`
//! (the slot finished resizing) and `onEntityEnteredFirstSlotFully` (the portrait
//! finished resizing; the bar unlocks).
//!
//! What arms the next first-slot selection, as the JS does: every `addEntity`, and the
//! removal of the entity at the front (counting entities already on their way out). It
//! picks the first entity not being removed, so an entity added mid-turn re-selects the
//! current one (whose turn then resumes), like the original. Headless, each step and
//! each removal takes one pump instead of an animation.

use squirrel::{SQVM, SqStrongRef};

use crate::UiCall;

pub(super) const MODULE: &str = "TurnSequenceBarModule";

struct Entry {
	id: i64,
	/// Fading out; dropped from the bar on the next pump.
	removing: bool,
}

/// A first-slot selection in progress: the entity and the callbacks still to come.
struct Selection {
	id: i64,
	next: Step,
}

#[derive(Clone, Copy)]
enum Step {
	Entered,
	EnteredFully,
}

#[derive(Default)]
pub(super) struct TurnSequenceBar {
	entries: Vec<Entry>,
	/// The first-slot selection timer is running.
	armed: bool,
	selection: Option<Selection>,
}

/// An entity id from a call's data: the value itself, or its `id` field.
fn entity_id(vm: &SQVM, data: &SqStrongRef) -> Option<i64> {
	data.get_value::<i64>(vm).or_else(|| data.get_member::<i64>(vm, "id"))
}

impl TurnSequenceBar {
	/// Apply one SQ→JS call to the bar.
	pub(super) fn apply(&mut self, vm: &SQVM, call: &UiCall) {
		match call.method.as_str() {
			"clear" => {
				self.entries.clear();
				self.armed = false;
			}
			"addEntity" => {
				let Some(id) = entity_id(vm, &call.data) else { return };
				if self.entries.iter().any(|e| e.id == id && !e.removing) {
					return;
				}
				self.entries.push(Entry { id, removing: false });
				self.armed = true;
			}
			"insertEntity" => {
				let Some(id) = entity_id(vm, &call.data) else { return };
				let index = call.data.get_member::<i64>(vm, "index").unwrap_or(1);
				if self.entries.iter().any(|e| e.id == id && !e.removing) || (index <= 0 && !self.entries.is_empty()) {
					return;
				}
				let at = (index.max(0) as usize).min(self.entries.len());
				self.entries.insert(at, Entry { id, removing: false });
			}
			"removeEntity" => {
				let Some(id) = entity_id(vm, &call.data) else { return };
				let Some(pos) = self.entries.iter().position(|e| e.id == id && !e.removing) else { return };
				if self.entries[..pos].iter().all(|e| e.removing) {
					self.armed = true;
				}
				self.entries[pos].removing = true;
			}
			_ => {}
		}
	}

	/// One pump: advance a selection in progress, finish removals, and select a new
	/// first entity if armed.
	pub(super) fn advance(&mut self, vm: &SQVM, receiver: &SqStrongRef) -> Result<(), String> {
		if let Some(sel) = self.selection.take() {
			match sel.next {
				Step::Entered => {
					notify(vm, receiver, "onEntityEnteredFirstSlot", sel.id)?;
					self.selection = Some(Selection {
						id: sel.id,
						next: Step::EnteredFully,
					});
				}
				Step::EnteredFully => notify(vm, receiver, "onEntityEnteredFirstSlotFully", sel.id)?,
			}
			return Ok(());
		}
		self.entries.retain(|e| !e.removing);
		if self.armed {
			if let Some(first) = self.entries.first().map(|e| e.id) {
				self.armed = false;
				// The JS queries the entity's data with it (the scripts start its turn); no
				// data (e.g. it died at its turn start) ends the selection there.
				let data = receiver
					.call_method_ret::<Option<SqStrongRef>, _>(vm, "onEntityEntersFirstSlot", (first,))
					.map_err(|e| format!("{MODULE}.onEntityEntersFirstSlot: {e}"))?;
				if matches!(data, Some(Some(_))) {
					self.selection = Some(Selection {
						id: first,
						next: Step::Entered,
					});
				}
			}
		}
		Ok(())
	}
}

fn notify(vm: &SQVM, receiver: &SqStrongRef, method: &str, id: i64) -> Result<(), String> {
	receiver.call_method(vm, method, (id,)).map_err(|e| format!("{MODULE}.{method}: {e}"))
}
