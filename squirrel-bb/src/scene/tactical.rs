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
	/// The highlighter's icons over tiles (an area attack's, a blocked shot's), drawn in
	/// the effects layer, over the units.
	pub overlay_icons: Vec<SceneOverlayIcon>,
	/// The footsteps of the move the player is about to make, standing on their tiles
	/// like units.
	pub path: Vec<ScenePathStep>,
	/// The marks on the tiles map objects stand on, shown with the
	/// `HighlightBlockedTiles` setting, in the effects layer (`zone_target_overlay` at
	/// alpha 0xa8, without the battle's light: `0x4cfedd` sets their
	/// `IgnoreAmbientColor` bit).
	pub blocked_tiles: Vec<SceneOverlayIcon>,
	/// The rain's drops (or snow's, or leaves'), in the effects layer: white sprites
	/// falling through the camera's view.
	pub rain: Vec<SceneRainDrop>,
	/// The rain's splats on the ground, each drawn with its tile's entities.
	pub rain_splats: Vec<SceneRainSplat>,
	/// The units' orientation overlays the camera drew this frame (its `render()`,
	/// from the battle state's `onRender`): in screen space, over the whole battle and
	/// under the UI.
	pub orientation: Vec<SceneOrientationOverlay>,
}

/// A unit's orientation overlay: the round portrait of a unit the map doesn't show
/// (on a tile above the camera's level, hidden behind a tile, or off screen), drawn
/// unscaled by the zoom (see `native::camera_overlays`).
#[derive(Clone, Debug, PartialEq)]
pub struct SceneOrientationOverlay {
	pub brush: String,
	/// Screen pixels from the centre (of the game's screen, the video mode), y up; the
	/// brush's offset is still to add.
	pub pos: (f32, f32),
	pub scale: f32,
	/// ARGB, its fade applied.
	pub color: u32,
}

/// A falling drop (`Tactical.getWeather().buildRain`).
#[derive(Clone, Debug, PartialEq)]
pub struct SceneRainDrop {
	pub brush: String,
	/// World units, y up.
	pub pos: (f32, f32),
	pub scale: f32,
	/// ARGB: white, its alpha fading in and out.
	pub color: u32,
}

/// A drop's splat on the ground.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneRainSplat {
	/// The tile under it, if on the map.
	pub tile: Option<usize>,
	pub brush: String,
	pub pos: (f32, f32),
	pub scale: f32,
	/// ARGB: white, fading out.
	pub color: u32,
	/// The frame of its brush it shows: `rainsplat_01` plays its four once (see
	/// `native::sprite_anim`).
	pub frame: u32,
}

/// An icon over a tile (`Tactical.getHighlighter().addOverlayIcon`).
#[derive(Clone, Debug, PartialEq)]
pub struct SceneOverlayIcon {
	pub brush: String,
	/// World units, y up.
	pub pos: (f32, f32),
	/// ARGB; its alpha fades out after a clear.
	pub color: u32,
	/// The tile whose sight it's drawn by, desaturated while the player can't see it:
	/// a blocked tile's mark (flag `0x100000`, `0x4b94bf`). An overlay icon has none
	/// (`addOverlayIcon` ignores its tile).
	pub tile: Option<usize>,
}

/// A footstep of the path preview (`navigator.buildVisualisation`).
#[derive(Clone, Debug, PartialEq)]
pub struct ScenePathStep {
	pub tile: usize,
	/// The tile's world position, level included (where a unit there would stand).
	pub pos: (f64, f64),
	/// `steps_<direction>`: the way the path enters the tile.
	pub brush: String,
	/// ARGB: pale yellow (`#e0e5a2`) while affordable, grey (`#777777`) past that.
	pub color: u32,
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
	/// Drawn without the battle's light (`IgnoreAmbientColor`): the icons, which the
	/// original flags so (`0x4bbe20`); attacks, sprite effects, projectiles and heads
	/// take the light.
	pub unlit: bool,
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
	/// The frame of its brush it shows (see `native::sprite_anim`).
	pub frame: u32,
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
	/// drawn.
	pub visible: bool,
	/// The player has seen it (`IsDiscovered`, bit 0 of `tile+0x10`). The original's tile
	/// pass skips a tile not discovered with everything on it (`0x557f93`, under the
	/// battle tile node's flag `0x20`, always set, `0x4ceee2`), so it stays black.
	pub discovered: bool,
	/// The player sees something on it that a tree or boulder in front of it (one to
	/// three rows down the map) fades for: a unit (`setPreventOcclusion`), items on the
	/// ground, or a step of the movement path shown (see `native::tactical_unveil`).
	pub unveils: bool,
	/// The highlighter's mark: the brush and colour (ARGB) drawn over the tile's
	/// terrain, under what stands on it (a zone of control, a skill's range, a target).
	pub highlight: Option<(String, u32)>,
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
	/// The frame of its brush it shows (see `native::sprite_anim`).
	pub frame: u32,
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
