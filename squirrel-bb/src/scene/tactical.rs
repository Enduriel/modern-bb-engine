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
	/// The units and map objects (trees, rocks, …) standing on tiles, in creation order.
	pub entities: Vec<SceneTacticalEntity>,
	/// The battle's light (`Tactical.getWeather().setAmbientLighting*`).
	pub light: crate::scene::SceneLight,
	/// The clouds or fog drifting over the battle (`Tactical.getWeather().buildCloudCover`).
	pub clouds: Vec<crate::scene::SceneCloud>,
	/// Tree tops are hidden (`Settings.getTempGameplaySettings().HideTrees`, the `T` key;
	/// a battle starts with `AlwaysHideTrees`).
	pub hide_trees: bool,
	/// Particle effects (`Tactical.spawnParticleEffect`), effect by effect.
	pub particles: Vec<crate::scene::SceneParticle>,
	/// The other effects over the battle (icons, attacks, sprite effects,
	/// projectiles), in the order they began.
	pub effects: Vec<SceneEffect>,
	/// What lies on the ground from effects, oldest first: pools
	/// (`Tactical.spawnPoolEffect`) and landed heads with their blood.
	pub pools: Vec<ScenePool>,
}

/// An effect over the battle, as drawn now.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneEffect {
	pub brush: String,
	/// Where its sprite's anchor is, in world units.
	pub pos: (f32, f32),
	pub scale: f32,
	/// Degrees, counter-clockwise.
	pub rotation: f32,
	/// ARGB, its fade in the alpha.
	pub color: u32,
	pub saturation: f32,
	pub mirrored: bool,
	/// Placed by its brush's offset (projectiles), or without it (icons, attacks and
	/// sprite effects, which the original flags so).
	pub brush_offset: bool,
}

/// A pool on the ground, as drawn now.
#[derive(Clone, Debug, PartialEq)]
pub struct ScenePool {
	/// The index of its tile in [`TacticalScene::tiles`].
	pub tile: usize,
	pub brush: String,
	pub pos: (f32, f32),
	pub scale: f32,
	/// Degrees, counter-clockwise.
	pub rotation: f32,
	/// ARGB.
	pub color: u32,
	pub saturation: f32,
	pub mirrored: bool,
}

/// A unit or map object on the battle map.
#[derive(Clone, Debug)]
pub struct SceneTacticalEntity {
	pub id: i64,
	/// Its position in world units, its tile's level included.
	pub pos: (f64, f64),
	/// The index of its tile in [`TacticalScene::tiles`].
	pub tile: Option<usize>,
	/// In the air between two tiles: drawn in the original's effects layer, after the
	/// tiles, not with the units on the map.
	pub flying: bool,
	/// Drawn on a tile the player can't see now (desaturated); off for actors, so the
	/// player never sees a unit there (`setVisibleInFogOfWar`).
	pub visible_in_fog: bool,
	/// A unit, rather than a map object.
	pub is_actor: bool,
	/// Its bars and status icons (`createOverlay`), as drawn now.
	pub overlay: Option<SceneOverlay>,
	/// Its sprites, back to front.
	pub sprites: Vec<SceneTacticalSprite>,
}

/// A unit's bars and status icons, as drawn now.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneOverlay {
	/// Each bar's value in 24ths (none at 0): head armour, body armour, hitpoints.
	pub bars: [i32; 3],
	/// Under each bar, what it showed before its last drop (in 24ths), and that
	/// twin's colour (ARGB) as it fades out.
	pub lost: [Option<(i32, u32)>; 3],
	/// The status icons (`setOverlayIcons`), brush names in order.
	pub icons: Vec<String>,
	/// How much the frame, bars and icons show, 0 to 1 (with the stats toggle off, it
	/// fades in and out after a change).
	pub alpha: f32,
}

/// One of a battle entity's named sprites.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneTacticalSprite {
	pub name: String,
	/// `""` for none.
	pub brush: String,
	/// ARGB (its alpha is the sprite's `Alpha`).
	pub color: u32,
	pub scale: f32,
	/// Degrees.
	pub rotation: f32,
	pub saturation: f32,
	/// `Highlight` (ARGB), blended over the sprite by `highlight_factor` (0: none): a
	/// hit's flash (see the shaker).
	pub highlight: u32,
	pub highlight_factor: f32,
	pub visible: bool,
	pub flipped: bool,
	/// From the entity's position, in world units with y up.
	pub offset: (f32, f32),
	pub ignore_ambient: bool,
	/// `setSpriteOcclusion`'s three values, for a tree's or boulder's top.
	pub occlusion: Option<(i64, i64, i64)>,
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
	/// The player sees it now (the player's bit in its visibility mask, `tile+0xc0 & 2`).
	/// Out of sight, a tile and what's on it draw desaturated, and units on it aren't
	/// drawn; that includes a tile nobody has seen yet.
	pub visible: bool,
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
