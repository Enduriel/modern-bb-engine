//! What a renderer draws of the world map, read out of the engine's state.
//!
//! The game logic keeps everything the scripts set on tiles, details and entity
//! sprites; [`Engine::world_scene`](crate::Engine::world_scene) copies it into these
//! plain values once per frame, so a renderer never touches the VM.

pub mod steps;
pub mod tactical;

pub use crate::native::world_camera::CameraView as SceneWorldCamera;
pub use crate::native::particles::SceneParticle;
pub use crate::native::world_clouds::SceneCloud;
pub use crate::native::world_waves::SceneWave;

/// The world map as last set by the scripts.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorldScene {
	/// Every tile, row by row (`col` fastest); only those on screen from
	/// [`Engine::world_scene_where`](crate::Engine::world_scene_where).
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
	/// The time of day's lighting.
	pub light: SceneLight,
	/// The clouds drifting over the map (`World.getWeather().buildCloudCover`).
	pub clouds: Vec<SceneCloud>,
	/// The birds flying over it (`World.getWeather().buildBirds`), in the effects layer.
	pub birds: Vec<SceneBird>,
	/// The footprints shown, oldest first.
	pub footprints: Vec<SceneFootprint>,
	/// Waves on the sea (`World.spawnWaveSprite`), oldest first.
	pub waves: Vec<SceneWave>,
	/// The camera (`World.getCamera()`): where the screen's centre is and its zoom.
	pub camera: SceneWorldCamera,
	/// Particle effects (`World.spawnParticleEffect`), effect by effect.
	pub particles: Vec<SceneParticle>,
}

/// What a [`WorldScene`] without its tiles is made from, to tell whether it would
/// differ from another without building it (see
/// [`Engine::world_scene_key`](crate::Engine::world_scene_key)): equal keys, equal
/// scenes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldSceneKey {
	/// The changes noted to the entities, clouds, birds, waves, particles, footprints and
	/// region names (see `world_scene_changes`).
	pub changes: u64,
	pub extent: (f32, f32),
	pub fog_of_war: bool,
	pub light: SceneLight,
	pub camera: SceneWorldCamera,
	/// Footprints show (the gameplay setting `ShowTracking`).
	pub tracking: bool,
	/// The virtual clock, which the footprints fade on (0 without a map). In full: as an
	/// `f32` it stopped telling frames apart after about 2^16 s of a campaign (its step
	/// longer than a 180 Hz frame).
	pub virtual_time: f64,
}

/// Which kind of entity a procedural image shows, by the handler its path names. Each
/// finds only its own kind's entities.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PortraitKind {
	/// `tacticalentity(...)`: a battle unit (also a brother in the roster), the
	/// original's only handler.
	#[default]
	Tactical,
	/// `worldentity(...)`: a world entity (a party, a location, a settlement). The
	/// original has no such handler; ours is a deliberate divergence
	/// (`ModernEngine.Version` 3).
	World,
}

/// What an entity's portrait draws (the UI's `coui://procedural/tacticalentity(...)`
/// and `worldentity(...)` images): its sprites, in the order they were added.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScenePortrait {
	/// The kind of entity, whose rules draw it (see bb-render's `portrait`).
	pub kind: PortraitKind,
	pub sprites: Vec<PortraitSprite>,
	/// Sprites sit at their offsets (`setAlwaysApplySpriteOffset`); otherwise all at
	/// the entity's anchor. A battle unit's only: a world entity's always do.
	pub apply_offsets: bool,
}

/// One of a portrait's sprites.
#[derive(Clone, Debug, PartialEq)]
pub struct PortraitSprite {
	pub name: String,
	/// `""` for none.
	pub brush: String,
	/// ARGB.
	pub color: u32,
	pub scale: f32,
	/// Degrees.
	pub rotation: f32,
	pub saturation: f32,
	pub flipped: bool,
	/// From the entity's anchor, in world units with y up.
	pub offset: (f32, f32),
	pub visible: bool,
	/// Drawn into portraits (`setSpriteRenderToTexture`).
	pub in_portrait: bool,
	/// Keeps its screen size at any zoom (a world entity's `setSpriteScaling(name,
	/// false)`: its selection ring, a location's banner); never a battle unit's.
	pub keeps_screen_size: bool,
}

/// A footprint (`World.spawnFootprint`).
#[derive(Clone, Debug, PartialEq)]
pub struct SceneFootprint {
	pub brush: std::rc::Rc<str>,
	pub pos: (f32, f32),
	pub scale: f32,
	/// 0 to 255.
	pub alpha: f32,
}

/// The world's lighting: the day phase's ambient colour and saturation, which lit
/// sprites, the water and the fog are multiplied by.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneLight {
	/// ARGB.
	pub color: u32,
	pub saturation: f32,
}

impl Default for SceneLight {
	/// Neutral: white, full saturation.
	fn default() -> Self {
		SceneLight {
			color: 0xffff_ffff,
			saturation: 1.0,
		}
	}
}

/// A region's name on the map (`World.spawnRegionText`).
#[derive(Clone, Debug, PartialEq)]
pub struct SceneRegionText {
	/// One glyph per byte, like [`SceneLabel::text`].
	pub text: std::rc::Rc<[u8]>,
	/// The scale the script gave (`0.5`, `0.75` or `1` by the region's size).
	pub scale: f32,
	/// Degrees, counter-clockwise.
	pub rotation: f32,
	/// Where it is centred, in world units.
	pub pos: (f32, f32),
	/// Its entity id (the original's is a scene entity, which takes one).
	pub id: i64,
}

/// One world tile.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneTile {
	pub col: i64,
	pub row: i64,
	/// The tile's centre in world units.
	pub pos: (f64, f64),
	/// Its `Const.World.TerrainType` (1 is ocean).
	pub terrain: i64,
	/// Uncovered from the fog of war (every tile is when fog of war is off).
	pub discovered: bool,
	/// The terrain brush (`""` if unset), shared with the tile (a frame's scene copies
	/// tens of thousands of them when zoomed out).
	pub brush: std::rc::Rc<str>,
	/// `IsFlipped`: the terrain's picture mirrored in place (badlands set it at random).
	pub flipped: bool,
	/// Its decorations, in spawn order.
	pub details: Vec<SceneDetail>,
}

/// A detail added to the map or removed from it, for a renderer that keeps the map from
/// frame to frame and follows it edit by edit (see `WorldGrid::take_map_edits`).
#[derive(Clone, Debug, PartialEq)]
pub enum MapEdit {
	/// Spawned on the tile at flat index `tile` (`row * width + col`, the order
	/// `WorldGrid::scene_map` gives the tiles in), after the tile's other details. As
	/// drawn when the edits were taken.
	Spawned { tile: usize, detail: SceneDetail },
	/// The detail with this id removed.
	Removed(i64),
	/// The terrain of the tile at flat index `tile` set (its brush or mirroring), as
	/// drawn when the edits were taken (`terrain`, without its details).
	Terrain { tile: usize, terrain: SceneTile },
}

/// A tile's detail as drawn (see `world_detail`).
#[derive(Clone, Debug, PartialEq)]
pub struct SceneDetail {
	/// Its entity id.
	pub id: i64,
	/// Not `Visible` (only the whole map's details include such, see
	/// `WorldGrid::scene_map`).
	pub hidden: bool,
	/// Its brush, shared with the detail.
	pub brush: std::rc::Rc<str>,
	/// Draw layer, a `Const.World.ZLevel`: higher is further back.
	pub z_level: i64,
	/// `Const.World.DetailType` bits.
	pub flags: i64,
	pub flipped: bool,
	/// The colour drawn, any fade applied: its alpha (0–255), and its red, green and
	/// blue (RGB).
	pub alpha: i64,
	pub tint: u32,
	/// The frame of its brush it shows.
	pub frame: u32,
	/// Degrees.
	pub rotation: f64,
	pub saturation: f64,
	pub scale: f64,
	pub ignore_ambient: bool,
	/// Where it is: its tile's centre plus its offset, in world units.
	pub pos: (f32, f32),
	/// Its draw order key (the original's Z).
	pub z: i32,
}

impl Default for SceneDetail {
	fn default() -> Self {
		SceneDetail {
			id: 0,
			hidden: false,
			brush: "".into(),
			z_level: 0,
			flags: 0,
			flipped: false,
			alpha: 255,
			tint: 0x00ff_ffff,
			frame: 0,
			rotation: 0.0,
			saturation: 1.0,
			scale: 1.0,
			ignore_ambient: false,
			pos: (0.0, 0.0),
			z: 0,
		}
	}
}

/// One world entity (a party, location, …).
#[derive(Clone, Debug, PartialEq)]
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
#[derive(Clone, Debug, PartialEq)]
pub struct SceneLabel {
	/// Shared with the entity (a frame's scene copies no text).
	pub name: std::rc::Rc<str>,
	/// One glyph per byte, as the original draws it: scripts' bytes, not necessarily
	/// UTF-8.
	pub text: std::rc::Rc<[u8]>,
	pub visible: bool,
	/// ARGB.
	pub color: u32,
	/// From the entity's position, in world units with y up.
	pub offset: (f64, f64),
}

/// One of an entity's named sprites.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneSprite {
	/// Shared with the entity, as the brush (a frame's scene copies no text).
	pub name: std::rc::Rc<str>,
	/// The brush drawn (`""` for none).
	pub brush: std::rc::Rc<str>,
	/// ARGB as drawn now (a fade in progress included).
	pub color: u32,
	pub scale: f64,
	pub visible: bool,
	/// From the entity's position, in world units with y up.
	pub offset: (f64, f64),
	pub flipped: bool,
	/// Degrees, counter-clockwise.
	pub rotation: f32,
	pub ignore_ambient: bool,
	pub scaling: bool,
	pub colorization: bool,
	/// The frame of its brush it shows (see `native::sprite_anim`).
	pub frame: u32,
}

/// A bird over the world map (see `native::world_birds`). It's drawn white at alpha
/// 0xa8, with a shadow: the same brush at alpha 0x10 (its colour wasn't read; the
/// clouds' shadows are black), 1.25 times as large, 150 units below.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneBird {
	pub brush: &'static str,
	/// World units, y up.
	pub pos: (f32, f32),
	/// Units per second.
	pub velocity: f32,
	/// Sort key in the effects layer.
	pub key: i64,
	/// The frame of its brush it shows (see `native::sprite_anim`).
	pub frame: u32,
}
