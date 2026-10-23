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
//! - `World.load()`: a save replaces a campaign that was in the world.
//!
//! A load from the main menu replaces none (the campaign left was collected as it was
//! left), so it doesn't collect: the pass would cost a full walk of the loaded campaign
//! (95 ms with a large mod corpus) to free what the load's own scripts
//! left in cycles (none in vanilla, 5–8 cycles, under 50 kB, with those mods); the next
//! discard frees them.
//!
//! These are moments behind a loading screen, so the pass costs nothing a player sees.
//! The host hears of each too ([`Discard`], `Engine::take_discards`): the renderer
//! drops the textures the discarded scene drew there, and the sound output the sounds it
//! kept for reuse.
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
	/// Whether a scene was discarded since the last frame's end.
	discarded: Rc<Cell<bool>>,
	discards: Rc<RefCell<Vec<Discard>>>,
}

impl Collector {
	/// Wrap the natives that discard object graphs so that each asks for a collection.
	/// Call once they are all installed (the wrappers call whatever is there then).
	/// `campaign` says whether the world holds a campaign (a load replaces it only then).
	pub(crate) fn install(vm: &mut SQVM, campaign: impl Fn() -> bool + 'static) -> Collector {
		let requested = Rc::new(Cell::new(false));
		let discarded = Rc::new(Cell::new(false));
		let discards: Rc<RefCell<Vec<Discard>>> = Rc::default();
		let native = |vm: &SQVM, table: &str, name: &str| {
			SqTable::root(vm).get::<SqTable>(table).and_then(|t| t.get::<SqStrongRef>(name))
		};
		for (table, discard) in [("Tactical", Discard::Battle), ("World", Discard::Campaign)] {
			let Some(inner) = native(vm, table, "clearScene") else { continue };
			let (flag, any, seen) = (requested.clone(), discarded.clone(), discards.clone());
			let clear = vm.make_closure(move |vm: &SQVM| -> Result<(), String> {
				inner.call(vm, ())?;
				flag.set(true);
				any.set(true);
				seen.borrow_mut().push(discard);
				Ok(())
			});
			if let Some(mut owner) = SqTable::root(vm).get::<SqTable>(table) {
				owner.set("clearScene", clear);
			}
		}
		if let Some(inner) = native(vm, "World", "load") {
			let (flag, any, seen) = (requested.clone(), discarded.clone(), discards.clone());
			let load = vm.make_closure(move |vm: &SQVM, file: String| -> Result<(), String> {
				let replaced = campaign();
				inner.call(vm, (file,))?;
				if replaced {
					flag.set(true);
				}
				any.set(true);
				seen.borrow_mut().push(Discard::Load);
				Ok(())
			});
			if let Some(mut world) = SqTable::root(vm).get::<SqTable>("World") {
				world.set("load", load);
			}
		}
		Collector { requested, discarded, discards }
	}

	/// The scenes discarded since the last call, in order.
	pub(crate) fn take_discards(&self) -> Vec<Discard> {
		std::mem::take(&mut self.discards.borrow_mut())
	}

	/// Collect if a discard asked for it since the last frame; whether any scene was
	/// discarded since then.
	pub(crate) fn collect_if_requested(&self, vm: &SQVM) -> bool {
		if self.requested.replace(false) {
			vm.collect_garbage();
		}
		self.discarded.replace(false)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A VM with stand-ins for the natives the collector wraps, and whether the world
	/// holds a campaign.
	fn collector() -> (SQVM, Collector, Rc<Cell<bool>>) {
		let mut vm = SQVM::new(1024);
		let load = vm.make_closure(|_file: String| {});
		let clear = vm.make_closure(|| {});
		let mut world = SqTable::new(&vm);
		world.set("load", load);
		world.set("clearScene", clear);
		SqTable::root(&vm).set("World", world);
		let campaign = Rc::new(Cell::new(false));
		let held = campaign.clone();
		let collector = Collector::install(&mut vm, move || held.get());
		(vm, collector, campaign)
	}

	#[test]
	fn a_load_collects_only_where_it_replaces_a_campaign() {
		let (vm, collector, campaign) = collector();
		// From the main menu: the campaign left was collected as it was left.
		vm.run_script(r#"World.load("a");"#, "test").unwrap();
		assert!(!collector.requested.get(), "a load from the menu doesn't collect");
		assert!(collector.collect_if_requested(&vm), "it is still a discard");
		assert_eq!(collector.take_discards(), [Discard::Load]);
		assert!(!collector.collect_if_requested(&vm), "nothing discarded since");

		// In the campaign: the old one's cycles go.
		campaign.set(true);
		vm.run_script(r#"World.load("b");"#, "test").unwrap();
		assert!(collector.requested.get(), "a load replacing a campaign collects");
		assert!(collector.collect_if_requested(&vm));
		assert!(!collector.requested.get());

		// Leaving the campaign always collects.
		campaign.set(false);
		vm.run_script(r#"World.clearScene();"#, "test").unwrap();
		assert!(collector.requested.get());
		assert!(collector.collect_if_requested(&vm));
		assert_eq!(collector.take_discards(), [Discard::Load, Discard::Campaign]);
	}
}
