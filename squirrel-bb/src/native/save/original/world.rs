//! What reading and writing the original's layout needs from the engine, step by step
//! in the original's order (see [`super`]). The engine implements it over its parts
//! (`crate::native::original_world`).

use squirrel::{SQVM, SqStrongRef};

use super::camera::{Camera, Clock};
use super::record::{Head, TacticalNative, WorldEntityPart};
use super::scene::Scene;
use super::sprite::Sprite;

/// The fixed rosters (the others are `createRoster`'s, by their id).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Roster {
	Player,
	Guests,
	Created(u32),
}

pub(crate) trait OriginalWorld {
	/// Empty the world, as the original's `World.clear` does before a load: every world
	/// entity, unit and roster goes (nothing runs for them).
	fn clear(&self, vm: &SQVM) -> Result<(), String>;

	/// The virtual time, in seconds.
	fn set_virtual_time(&self, seconds: f64);

	/// The map: its size, tiles, terrain, decorations and region names, and the
	/// ambient lighting.
	fn load_map(&self, vm: &SQVM, scene: &Scene) -> Result<(), String>;

	/// A world entity of script `path`: created (`create`, `onInit`, `onAfterInit`)
	/// with its saved id, position and flags, then its saved sprites put back. `top`:
	/// it was in the layer drawn on top (`RenderedTop`).
	fn load_entity(&self, vm: &SQVM, path: &str, top: bool, head: &Head, part: &WorldEntityPart) -> Result<(), String>;

	/// A footprint: where, its sprite, when its stay ends, its kind.
	fn load_footprint(&self, head: &Head, sprite: &Sprite, expires: f32, kind: u16);

	fn load_camera(&self, camera: &Camera);
	fn load_clock(&self, clock: &Clock);

	/// A unit of script `path`: created (`create`, `onInit`, `onAfterInit`) with its
	/// saved id, position and sprites. Its script is returned, for its `onDeserialize`.
	fn load_unit(&self, vm: &SQVM, path: &str, head: &Head, native: &TacticalNative) -> Result<SqStrongRef, String>;

	/// Append a loaded unit to `roster`.
	fn add_unit(&self, roster: Roster, unit: SqStrongRef);

	/// Every world entity's id and script, by id descending.
	fn entity_scripts(&self) -> Vec<(u32, SqStrongRef)>;

	/// The fog of war's mask.
	fn load_fog(&self, mask: Vec<u8>) -> Result<(), String>;
}
