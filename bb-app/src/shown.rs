//! Whether a frame would show what the last one showed, so the window can leave the
//! last one up instead of drawing and presenting the same pixels again.
//!
//! A frame is a function of what [`Look`] holds: the frame's size, the page's latest
//! paint, the drawn cursor, the fonts, and under the UI either nothing (the main menu),
//! the world map's scene with the view and the clocks its drawing reads, or a battle's
//! scene with its view. The world map's water drifts with virtual time (still while the
//! game is paused) and its fog of war with real time in steps
//! (`bb_render::fog_drift_time`), so a view with fog on it looks the same until the
//! fog's next step. A battle's drawing reads no clock: whatever moves in it with time
//! (animations, fades, rain, clouds, particles, a flashing highlight) is in its scene as
//! drawn now, so a moving battle differs from frame to frame by its scene. The scripts
//! still run every frame: only the drawing is left out.

use squirrel_bb::scene::WorldSceneKey;
use squirrel_bb::scene::tactical::TacticalScene;

/// What one frame shows, as far as telling two frames apart goes.
#[derive(Debug, PartialEq)]
pub struct Look {
	/// The frame's size in pixels.
	pub size: (u32, u32),
	/// The page's paints so far: a new paint is a new picture of it.
	pub page_paints: u64,
	/// The drawn cursor, if any.
	pub cursor: Option<CursorLook>,
	/// The fonts the renderer has (mods register theirs as they boot).
	pub fonts: usize,
	pub under: Under,
}

/// The drawn cursor: its brush, where, and which frame of its animation.
#[derive(Debug, PartialEq)]
pub struct CursorLook {
	pub brush: String,
	pub at: [f32; 2],
	/// The animation frame's picture (`BrushFrame` uv and rect), the same for a still
	/// cursor at any time.
	pub frame: Option<([f32; 4], [f32; 4])>,
}

/// What is drawn under the page.
#[derive(Debug, PartialEq)]
pub enum Under {
	/// Nothing: the main menu (no map shows).
	Nothing,
	/// The world map: what its scene (without the tiles, which the renderer keeps) is
	/// made from, compared without building it (`Engine::world_scene_key`), the view
	/// (centre, zoom), the virtual clock the water drifts with (0 without a map, where
	/// no water is drawn), the time the fog of war is drawn at where it shows
	/// (`bb_render::fog_drift_time`), and the map's revision (its terrain and details; a
	/// detail spawned, removed or whose look changed draws anyway, see
	/// `Game::draw_if_changed`).
	World {
		scene: WorldSceneKey,
		view: ([f32; 2], f32),
		virtual_time: f32,
		fog: Option<f32>,
		map_revision: u64,
	},
	/// A battle: its scene (the map, the units, the effects, the weather, the
	/// orientation overlays, all as drawn now), the view (centre, zoom), the frame
	/// pixels to a screen pixel the orientation overlays are placed by, and the depth
	/// of field.
	Battle {
		scene: Box<TacticalScene>,
		view: ([f32; 2], f32),
		fit: f32,
		depth_of_field: bool,
	},
}

/// The last frame drawn, if it can be compared.
#[derive(Default)]
pub struct Shown {
	last: Option<Look>,
}

impl Shown {
	/// Whether `look` is what the last frame drawn showed.
	pub fn same(&self, look: Option<&Look>) -> bool {
		matches!((&self.last, look), (Some(last), Some(look)) if last == look)
	}

	/// A frame was drawn showing `look`; `None` when it can't be compared (it changes
	/// with time, or drew without something still loading), so the next frame is drawn.
	pub fn drawn(&mut self, look: Option<Look>) {
		self.last = look;
	}

	/// The next frame is drawn whatever it shows (the window's surface was replaced).
	pub fn forget(&mut self) {
		self.last = None;
	}
}

/// The world rectangle `[left, right, bottom, top]` a view centred on `center` at
/// `zoom` (screen pixels per world unit) shows on a `width` × `height` frame.
pub fn view_rect(center: [f32; 2], zoom: f32, width: u32, height: u32) -> [f32; 4] {
	let half_w = width as f32 / zoom / 2.0;
	let half_h = height as f32 / zoom / 2.0;
	[center[0] - half_w, center[0] + half_w, center[1] - half_h, center[1] + half_h]
}

#[cfg(test)]
mod tests {
	use super::*;

	fn look(paints: u64) -> Look {
		Look {
			size: (1920, 1080),
			page_paints: paints,
			cursor: None,
			fonts: 0,
			under: Under::Nothing,
		}
	}

	#[test]
	fn only_a_comparable_look_matches_the_last() {
		let mut shown = Shown::default();
		assert!(!shown.same(Some(&look(1))), "nothing drawn yet");
		shown.drawn(Some(look(1)));
		assert!(shown.same(Some(&look(1))));
		assert!(!shown.same(Some(&look(2))), "the page painted");
		assert!(!shown.same(None), "a frame that changes with time");
		shown.drawn(None);
		assert!(!shown.same(Some(&look(1))), "after one that couldn't be compared");
		shown.drawn(Some(look(1)));
		shown.forget();
		assert!(!shown.same(Some(&look(1))));
	}

	#[test]
	fn a_battle_matches_while_its_scene_and_view_do() {
		let battle = |scene: TacticalScene, center: f32| Look {
			under: Under::Battle {
				scene: Box::new(scene),
				view: ([center, 0.0], 1.0),
				fit: 1.0,
				depth_of_field: false,
			},
			..look(1)
		};
		let mut shown = Shown::default();
		shown.drawn(Some(battle(TacticalScene::default(), 0.0)));
		assert!(shown.same(Some(&battle(TacticalScene::default(), 0.0))));
		assert!(!shown.same(Some(&battle(TacticalScene::default(), 1.0))), "the camera moved");
		let raining = TacticalScene {
			rain: vec![squirrel_bb::scene::tactical::SceneRainDrop {
				brush: "rain".into(),
				pos: (1.0, 2.0),
				scale: 1.0,
				color: 0xffff_ffff,
			}],
			..TacticalScene::default()
		};
		assert!(!shown.same(Some(&battle(raining, 0.0))), "a drop fell");
	}

	#[test]
	fn a_view_covers_the_frame_over_the_zoom() {
		assert_eq!(view_rect([100.0, 50.0], 2.0, 400, 200), [0.0, 200.0, 0.0, 100.0]);
	}
}
