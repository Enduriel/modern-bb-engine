//! What a renderer draws of a battle, read out of the engine's state
//! ([`Engine::tactical_scene`](crate::Engine::tactical_scene)).

use std::collections::HashMap;

pub use crate::native::tactical::TileTransition as SceneTransition;

/// The battle map as last set by the scripts.
#[derive(Clone, Debug, Default)]
pub struct TacticalScene {
	/// Square size of the map.
	pub width: i64,
	pub height: i64,
	/// Every tile, row by row (`col` fastest).
	pub tiles: Vec<SceneTacticalTile>,
	/// Each terrain brush's transition set (`Tactical.setTransitions`), by brush name.
	pub transitions: HashMap<String, SceneTransition>,
	pub camera: SceneCamera,
}

/// One battle tile.
#[derive(Clone, Debug)]
pub struct SceneTacticalTile {
	pub col: i64,
	pub row: i64,
	/// The tile's centre on the ground, in world units (its level not included).
	pub pos: (f64, f64),
	pub level: i64,
	/// `Const.Tactical.TerrainType`, and the subtype.
	pub terrain: i64,
	pub subtype: i64,
	/// Which of two neighbouring tiles' transitions draws over the other.
	pub blend_priority: i64,
	/// The terrain brush (`""` if unset).
	pub brush: String,
	/// ARGB (`tile.setColor`).
	pub color: u32,
	/// Seen before by the player but not now.
	pub fogged: bool,
	/// Its details, in spawn order.
	pub details: Vec<SceneTacticalDetail>,
}

/// A detail on a battle tile (`tile.spawnDetail`).
#[derive(Clone, Debug, PartialEq)]
pub struct SceneTacticalDetail {
	pub brush: String,
	pub flags: i64,
	pub mirrored: bool,
	/// From the tile's centre, in world units with y up.
	pub offset: (f32, f32),
	/// Degrees, counter-clockwise.
	pub rotation: f32,
	pub scale: f32,
	pub saturation: f32,
	pub brightness: f32,
	/// ARGB as drawn now (a fade in progress included).
	pub color: u32,
}

/// The battle camera.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SceneCamera {
	/// The world position at the centre of the screen.
	pub pos: (f64, f64),
	/// World units per screen pixel.
	pub zoom: f64,
	/// The level it looks from; tiles above it are cut away.
	pub level: i64,
	pub flipped: bool,
}
