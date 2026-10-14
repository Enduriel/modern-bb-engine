//! `World.clearScene()` (`0x4efcd0`, disassembled), which the world state's
//! `onFinish` calls on the way back to the main menu. It empties the world, so the next
//! campaign starts from nothing:
//!
//! - every world entity, the thinker list and the footprints;
//! - the scene's layers (details, entities, effects: clouds, birds, waves, particle
//!   effects)
//!   and its texts (the region names);
//! - the player's, the guests' and the temporary rosters, and every roster
//!   `createRoster` made;
//! - the clock (day 1, 2:00), the speed, the player's vision radius and the global
//!   visibility (1.0), and fog of war (on);
//! - `Time`'s virtual time (0) and speed (1).
//!
//! The tiles stay (worldgen's `clearTiles` and `resizeScene` reset them), and so does
//! the entity id counter, which every engine object draws from.
//!
//! The original then collects reference cycles (`sq_collectgarbage`, `0x4eff63`); the
//! engine does at the end of the frame (see `collect`).

use squirrel::{SQVM, SqStrongRef, SqTable};

use super::particles::ParticlesHandle;
use super::world_clouds::Clouds;
use super::world_waves::Waves;
use super::{TacticalEntities, TimeService, WorldEntities, WorldGrid};

/// What `World.clearScene()` clears beyond the clock.
pub(crate) struct WorldScene {
	pub entities: WorldEntities,
	pub grid: WorldGrid,
	pub units: TacticalEntities,
	pub time: TimeService,
	pub clouds: Clouds,
	pub birds: super::world_birds::Birds,
	pub waves: Waves,
	pub particles: ParticlesHandle,
}

/// Replace `World.clearScene` (installed with the clock, which it keeps resetting) with
/// the whole of it. Call after every `World` native is installed.
pub(crate) fn install(vm: &mut SQVM, scene: WorldScene) {
	let clock = SqTable::root(vm)
		.get::<SqTable>("World")
		.and_then(|w| w.get::<SqStrongRef>("clearScene"));
	let clear = vm.make_closure(move |vm: &SQVM| -> Result<(), String> {
		if let Some(clock) = &clock {
			clock.call(vm, ())?;
		}
		scene.entities.clear_scene();
		scene.grid.clear_scene();
		scene.units.clear_for_load();
		scene.time.reset_virtual();
		scene.clouds.borrow_mut().clear();
		scene.birds.borrow_mut().clear();
		scene.waves.borrow_mut().clear();
		scene.particles.borrow_mut().clear();
		Ok(())
	});
	if let Some(mut world) = SqTable::root(vm).get::<SqTable>("World") {
		world.set("clearScene", clear);
	}
}
