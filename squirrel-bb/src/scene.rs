//! What a renderer draws of the world map, read out of the engine's state.
//!
//! The game logic keeps everything the scripts set on tiles, details and entity
//! sprites; [`Engine::world_scene`](crate::Engine::world_scene) copies it into these
//! plain values once per frame, so a renderer never touches the VM.

pub use crate::native::world_detail::DetailData as SceneDetail;

/// The world map as last set by the scripts.
#[derive(Clone, Debug, Default)]
pub struct WorldScene {
	/// Every tile, row by row (`col` fastest).
	pub tiles: Vec<SceneTile>,
	/// Every live entity, in spawn order.
	pub entities: Vec<SceneEntity>,
	/// The map's extent in world units, from the origin to the centre of the tile past
	/// its last corner (`(0, 0)` before there is a map). The fog of war's mask and the
	/// water texture span it.
	pub extent: (f32, f32),
	/// Fog of war is on (`World.setFogOfWar`).
	pub fog_of_war: bool,
	/// Region names, in spawn order.
	pub region_texts: Vec<SceneRegionText>,
}

/// A region's name on the map (`World.spawnRegionText`).
#[derive(Clone, Debug, PartialEq)]
pub struct SceneRegionText {
	pub text: String,
	/// The scale the script gave (`0.5`, `0.75` or `1` by the region's size).
	pub scale: f32,
	/// Degrees, counter-clockwise.
	pub rotation: f32,
	/// Where it is centred, in world units.
	pub pos: (f32, f32),
}

/// One world tile.
#[derive(Clone, Debug)]
pub struct SceneTile {
	pub col: i64,
	pub row: i64,
	/// The tile's centre in world units.
	pub pos: (f64, f64),
	/// Its `Const.World.TerrainType` (1 is ocean).
	pub terrain: i64,
	/// Uncovered from the fog of war (every tile is when fog of war is off).
	pub discovered: bool,
	/// The terrain brush (`""` if unset).
	pub brush: String,
	/// Its decorations, in spawn order.
	pub details: Vec<SceneDetail>,
}

/// One world entity (a party, location, …).
#[derive(Clone, Debug)]
pub struct SceneEntity {
	pub id: i64,
	/// Position in world units.
	pub pos: (f64, f64),
	/// Shown to the player: not faded out by fog of war or discovery.
	pub shown: bool,
	/// Its sprites, in the order they were added (back to front).
	pub sprites: Vec<SceneSprite>,
	/// Its text labels (e.g. a settlement's name).
	pub labels: Vec<SceneLabel>,
}

/// One of an entity's text labels.
#[derive(Clone, Debug)]
pub struct SceneLabel {
	pub name: String,
	pub text: String,
	pub visible: bool,
	/// ARGB.
	pub color: u32,
	/// From the entity's position, in world units with y up.
	pub offset: (f64, f64),
}

/// One of an entity's named sprites.
#[derive(Clone, Debug)]
pub struct SceneSprite {
	pub name: String,
	/// The brush drawn (`""` for none).
	pub brush: String,
	pub alpha: f64,
	pub scale: f64,
	pub visible: bool,
	/// From the entity's position, in world units with y up.
	pub offset: (f64, f64),
	pub flipped: bool,
	pub ignore_ambient: bool,
	pub scaling: bool,
	pub colorization: bool,
}
