//! When the engine frees the script objects only reference cycles keep alive.
//!
//! Reference counting frees an object as soon as nothing refers to it, but never a cycle:
//! a battle's units, their AI agents and behaviours point at each other, and a campaign's
//! managers, screens and entities do too. Squirrel's collector (`sq_collectgarbage`) frees
//! them, at the cost of one pass over every live object.
//!
//! The engine collects where the game discards such a graph, at the end of the frame it
//! happens in (once the scripts have let go of it):
//! - `Tactical.clearScene()`: a battle ends (the tactical state's `onFinish`);
//! - `World.clearScene()`: a campaign is left for the main menu;
//! - `World.load()`: a save replaces the campaign.
//!
//! These are moments behind a loading screen, so the pass costs nothing a player sees.
//! The host hears of each too ([`Discard`], `Engine::take_discards`): the renderer
//! drops the textures the discarded scene drew there.
//! The original collects at the end of `World.clearScene` and on a 60 s clock in its
//! frame loop (`0x4eff63`, `0x5394c0`); the difference is an intentional divergence
//! (`docs/differences.md`).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use squirrel::{SQVM, SqStrongRef, SqTable};

/// A scene the game discarded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Discard {
	/// `Tactical.clearScene()`: a battle ended.
	Battle,
	/// `World.clearScene()`: the campaign was left.
	Campaign,
	/// `World.load()`: a save replaced the campaign.
	Load,
}

/// A collection the next frame's end owes, the discards since the host last asked, and
/// the natives that ask for them.
pub(crate) struct Collector {
	requested: Rc<Cell<bool>>,
	discards: Rc<RefCell<Vec<Discard>>>,
}

impl Collector {
	/// Wrap the natives that discard object graphs so that each asks for a collection.
	/// Call once they are all installed (the wrappers call whatever is there then).
	pub(crate) fn install(vm: &mut SQVM) -> Collector {
		let requested = Rc::new(Cell::new(false));
		let discards: Rc<RefCell<Vec<Discard>>> = Rc::default();
		let native = |vm: &SQVM, table: &str, name: &str| {
			SqTable::root(vm).get::<SqTable>(table).and_then(|t| t.get::<SqStrongRef>(name))
		};
		for (table, discard) in [("Tactical", Discard::Battle), ("World", Discard::Campaign)] {
			let Some(inner) = native(vm, table, "clearScene") else { continue };
			let (flag, seen) = (requested.clone(), discards.clone());
			let clear = vm.make_closure(move |vm: &SQVM| -> Result<(), String> {
				inner.call(vm, ())?;
				flag.set(true);
				seen.borrow_mut().push(discard);
				Ok(())
			});
			if let Some(mut owner) = SqTable::root(vm).get::<SqTable>(table) {
				owner.set("clearScene", clear);
			}
		}
		if let Some(inner) = native(vm, "World", "load") {
			let (flag, seen) = (requested.clone(), discards.clone());
			let load = vm.make_closure(move |vm: &SQVM, file: String| -> Result<(), String> {
				inner.call(vm, (file,))?;
				flag.set(true);
				seen.borrow_mut().push(Discard::Load);
				Ok(())
			});
			if let Some(mut world) = SqTable::root(vm).get::<SqTable>("World") {
				world.set("load", load);
			}
		}
		Collector { requested, discards }
	}

	/// The scenes discarded since the last call, in order.
	pub(crate) fn take_discards(&self) -> Vec<Discard> {
		std::mem::take(&mut self.discards.borrow_mut())
	}

	/// Collect if a discard asked for it since the last frame.
	pub(crate) fn collect_if_requested(&self, vm: &SQVM) {
		if self.requested.replace(false) {
			vm.collect_garbage();
		}
	}
}
