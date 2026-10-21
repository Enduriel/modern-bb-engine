//! [`CloudCover`]: the clouds drifting over the world map (`World.getWeather()
//! .buildCloudCover(settings)`), as the original keeps them (disassembled).
//!
//! `buildCloudCover` (`0x4fe750`) replaces the clouds with `rand(MinClouds,
//! MaxClouds)` new ones. Each gets:
//! - a brush: one of `world_clouds_01`…`06` at random for the default type, or
//!   `foggy_weather_01` for the others;
//! - a position: uniform over the map (the centres of its first and last tiles),
//!   stretched 2000 units upwind; a `StaticFog` cover stays inside the map;
//! - a scale, an alpha and a velocity uniform in their settings' ranges (no draw when
//!   a range is empty), and a sort key of `rand(0, 1000) - 10000`.
//!
//! They drift along the fixed wind, `(-1, -0.5)` normalised (west and a little
//! south). Every `World.update()` (`0x4fdc30`) moves each by its velocity times the
//! frame's real delta, but only while the game's virtual speed isn't zero. A cloud
//! that leaves the map plus a 2000-unit margin downwind is replaced by a new one on
//! the map's top or right edge (at random), moved 2000 units upwind.
//!
//! Each cloud also casts a shadow (`0x4e3ed0`): black, a fifth of its alpha, 1.25
//! times its scale, 500 units times its scale below it, and drawn just under it.
//!
//! The original draws all this from a generator seeded from OS entropy at startup
//! (`0x483ca4`, `std::random_device`), never the scripts' stream. We use the engine's
//! own random stream ([`EngineRng`]), which is just as unpredictable unless a seed is
//! configured.

use std::cell::RefCell;
use std::rc::Rc;

use super::EngineRng;

/// The default type's brushes (`0x4840cc`…).
const CLOUD_BRUSHES: [&str; 6] = [
	"world_clouds_01",
	"world_clouds_02",
	"world_clouds_03",
	"world_clouds_04",
	"world_clouds_05",
	"world_clouds_06",
];

/// The other types' brush.
const FOG_BRUSH: &str = "foggy_weather_01";

/// `CloudType.StaticFog`: placed on the map, without the upwind margin.
const STATIC_FOG: i64 = 2;

/// How far past the map clouds reach and respawn.
const MARGIN: f32 = 2000.0;

/// The settings a script fills in (`createCloudSettings()`), with the original's
/// defaults (`0x4fd840`).
#[derive(Clone, Debug, PartialEq)]
pub struct CloudSettings {
	pub min_clouds: i64,
	pub max_clouds: i64,
	pub min_velocity: f32,
	pub max_velocity: f32,
	pub min_alpha: f32,
	pub max_alpha: f32,
	pub min_scale: f32,
	pub max_scale: f32,
	/// `CloudType`.
	pub kind: i64,
	/// ARGB.
	pub color: u32,
}

impl Default for CloudSettings {
	fn default() -> Self {
		CloudSettings {
			min_clouds: 1,
			max_clouds: 1,
			min_velocity: 0.0,
			max_velocity: 0.0,
			min_alpha: 1.0,
			max_alpha: 1.0,
			min_scale: 1.0,
			max_scale: 1.0,
			kind: 0,
			color: 0xffff_ffff,
		}
	}
}

/// One cloud, as a renderer reads it.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneCloud {
	pub brush: &'static str,
	/// World units.
	pub pos: (f32, f32),
	pub scale: f32,
	/// 0 to 1.
	pub alpha: f32,
	/// ARGB (its alpha is replaced by `alpha`).
	pub color: u32,
	/// Units per second.
	pub velocity: f32,
	/// Sort key in the effects layer; its shadow's is 2000 more.
	pub key: i64,
}

/// The map's first and last tile centres, which clouds are spread over.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapSpan {
	pub min: (f32, f32),
	pub max: (f32, f32),
}

/// The clouds over the world map.
pub struct CloudCover {
	rng: EngineRng,
	settings: CloudSettings,
	span: Option<MapSpan>,
	/// The wind, normalised.
	wind: (f32, f32),
	clouds: Vec<SceneCloud>,
	/// Counts the covers built and cleared (for a scene holding the clouds, see
	/// `scene::steps`).
	generation: u64,
}

/// A shared handle (the weather natives and the world update both use it).
pub type Clouds = Rc<RefCell<CloudCover>>;

impl CloudCover {
	pub fn new(rng: EngineRng) -> Self {
		// The original normalises (-1, -0.5) as -1 / sqrt(1.25) and -0.5 / sqrt(1.25).
		let k = 1.0 / 1.25f64.sqrt() as f32;
		CloudCover {
			rng,
			settings: CloudSettings::default(),
			span: None,
			wind: (-k, -0.5 * k),
			clouds: Vec::new(),
			generation: 0,
		}
	}

	/// A new one for each cover built or cleared.
	pub fn generation(&self) -> u64 {
		self.generation
	}

	/// How fast the fastest cloud drifts, in world units a second of the game's clock
	/// (none without a map).
	pub fn fastest(&self) -> f32 {
		if self.span.is_none() {
			return 0.0;
		}
		self.clouds.iter().map(|c| c.velocity.abs()).fold(0.0, f32::max)
	}

	/// The clouds, in no particular order.
	pub fn clouds(&self) -> &[SceneCloud] {
		&self.clouds
	}

	/// `buildCloudCover`: replace the clouds with a new cover over `span`.
	pub fn build(&mut self, settings: CloudSettings, span: MapSpan) {
		super::world_scene_changes::note();
		self.generation += 1;
		self.clouds.clear();
		let count = self.rng.range(settings.min_clouds, settings.max_clouds);
		self.settings = settings;
		self.span = Some(span);
		for _ in 0..count.max(0) {
			let brush = self.brush();
			let pos = if self.settings.kind == STATIC_FOG {
				let y = self.rng.between(span.min.1, span.max.1);
				(self.rng.between(span.min.0, span.max.0), y)
			} else {
				// The margin is on the upwind side of each axis.
				let (lo_x, hi_x) = self.upwind(self.wind.0);
				let (lo_y, hi_y) = self.upwind(self.wind.1);
				let y = self.rng.between(span.min.1 - lo_y, span.max.1 + hi_y);
				(self.rng.between(span.min.0 - lo_x, span.max.0 + hi_x), y)
			};
			let cloud = self.cloud(brush, pos);
			self.clouds.push(cloud);
		}
	}

	/// No clouds (a cover built without a map).
	pub fn clear(&mut self) {
		super::world_scene_changes::note();
		self.generation += 1;
		self.clouds.clear();
		self.span = None;
	}

	/// One `World.update()`: `delta` real seconds, or nothing when the game's virtual
	/// speed is zero.
	pub fn advance(&mut self, delta: f32, virtual_speed: f32) {
		let Some(span) = self.span else { return };
		let delta = if virtual_speed != 0.0 { delta } else { 0.0 };
		if delta != 0.0 && !self.clouds.is_empty() {
			super::world_scene_changes::note();
		}
		let (wx, wy) = self.wind;
		let bounds = (
			span.min.0 - MARGIN,
			span.min.1 - MARGIN,
			span.max.0 + MARGIN,
			span.max.1 + MARGIN,
		);
		let mut gone = 0;
		self.clouds.retain_mut(|c| {
			c.pos.0 += wx * c.velocity * delta;
			c.pos.1 += wy * c.velocity * delta;
			let out = (wx <= 0.0 && c.pos.0 < bounds.0)
				|| (wx >= 0.0 && c.pos.0 > bounds.2)
				|| (wy <= 0.0 && c.pos.1 < bounds.1)
				|| (wy >= 0.0 && c.pos.1 > bounds.3);
			gone += usize::from(out);
			!out
		});
		for _ in 0..gone {
			let brush = self.brush();
			let (lo_x, hi_x) = self.upwind(wx);
			let (lo_y, hi_y) = self.upwind(wy);
			// Along the top edge or down the right one, then upwind.
			let (x, y) = if self.rng.range(1, 2) == 1 {
				(self.rng.between(span.min.0 - lo_x, span.max.0 + hi_x), span.max.1)
			} else {
				(span.max.0, self.rng.between(span.min.1 - lo_y, span.max.1 + hi_y))
			};
			let cloud = self.cloud(brush, (x - wx * MARGIN, y - wy * MARGIN));
			self.clouds.push(cloud);
		}
	}

	fn brush(&self) -> &'static str {
		if self.settings.kind == 0 {
			CLOUD_BRUSHES[self.rng.range(0, CLOUD_BRUSHES.len() as i64 - 1) as usize]
		} else {
			FOG_BRUSH
		}
	}

	/// The margins below and above an axis's span for wind component `w`: on the
	/// upwind side only, and only for moving clouds.
	fn upwind(&self, w: f32) -> (f32, f32) {
		let margin = if self.settings.max_velocity != 0.0 { MARGIN } else { 0.0 };
		if w >= 0.0 { (margin, 0.0) } else { (0.0, margin) }
	}

	fn cloud(&self, brush: &'static str, pos: (f32, f32)) -> SceneCloud {
		let s = &self.settings;
		let scale = self.rng.between(s.min_scale, s.max_scale);
		let alpha = self.rng.between(s.min_alpha, s.max_alpha);
		let velocity = self.rng.between(s.min_velocity, s.max_velocity);
		SceneCloud {
			brush,
			pos,
			scale,
			alpha,
			color: s.color,
			velocity,
			key: self.rng.range(0, 1000) - 10_000,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn world_settings() -> CloudSettings {
		// world_state.setupWeather.
		CloudSettings {
			min_clouds: 70,
			max_clouds: 70,
			min_velocity: 35.0,
			max_velocity: 65.0,
			min_alpha: 0.8,
			max_alpha: 0.8,
			min_scale: 1.0,
			max_scale: 1.25,
			..CloudSettings::default()
		}
	}

	const SPAN: MapSpan = MapSpan {
		min: (120.0, 60.0),
		max: (25140.0, 20400.0),
	};

	#[test]
	fn a_cover_spreads_over_the_map_and_upwind() {
		let mut c = CloudCover::new(EngineRng::seeded(1));
		c.build(world_settings(), SPAN);
		assert_eq!(c.clouds().len(), 70);
		for cloud in c.clouds() {
			assert!(CLOUD_BRUSHES.contains(&cloud.brush));
			// The wind blows west and south, so the margin is east and north.
			assert!((120.0..25140.0 + MARGIN).contains(&cloud.pos.0), "{cloud:?}");
			assert!((60.0..20400.0 + MARGIN).contains(&cloud.pos.1), "{cloud:?}");
			assert!((1.0..1.25).contains(&cloud.scale));
			assert_eq!(cloud.alpha, 0.8);
			assert!((35.0..65.0).contains(&cloud.velocity));
			assert!((-10_000..=-9_000).contains(&cloud.key));
		}
	}

	#[test]
	fn clouds_drift_with_the_wind_while_the_game_runs() {
		let mut c = CloudCover::new(EngineRng::seeded(2));
		c.build(world_settings(), SPAN);
		let before = c.clouds()[0].clone();
		c.advance(1.0, 0.0);
		assert_eq!(c.clouds()[0], before, "the virtual speed is zero: no drift");
		c.advance(2.0, 4.0);
		let after = &c.clouds()[0];
		// Two real seconds at its velocity, whatever the game speed.
		let moved = ((after.pos.0 - before.pos.0).powi(2) + (after.pos.1 - before.pos.1).powi(2)).sqrt();
		assert!((moved - 2.0 * before.velocity).abs() < 1e-2, "{moved}");
		assert!(after.pos.0 < before.pos.0 && after.pos.1 < before.pos.1);
	}

	#[test]
	fn a_cloud_past_the_downwind_edge_comes_back_upwind() {
		let mut c = CloudCover::new(EngineRng::seeded(3));
		c.build(
			CloudSettings {
				min_clouds: 1,
				max_clouds: 1,
				min_velocity: 100.0,
				max_velocity: 100.0,
				..CloudSettings::default()
			},
			SPAN,
		);
		// Long enough to cross the whole map and margin.
		c.advance(400.0, 1.0);
		assert_eq!(c.clouds().len(), 1);
		let respawned = &c.clouds()[0];
		// On the top edge or the right one, then 2000 units upwind (east and north).
		let k = 1.0 / 1.25f32.sqrt();
		let top = (respawned.pos.1 - (20400.0 + 2000.0 * 0.5 * k)).abs() < 1e-2;
		let right = (respawned.pos.0 - (25140.0 + 2000.0 * k)).abs() < 1e-2;
		assert!(top || right, "{respawned:?}");
	}

	#[test]
	fn a_static_fog_stays_on_the_map() {
		let mut c = CloudCover::new(EngineRng::seeded(4));
		c.build(
			CloudSettings {
				min_clouds: 20,
				max_clouds: 20,
				kind: STATIC_FOG,
				..CloudSettings::default()
			},
			SPAN,
		);
		for cloud in c.clouds() {
			assert_eq!(cloud.brush, FOG_BRUSH);
			assert!((120.0..25140.0).contains(&cloud.pos.0) && (60.0..20400.0).contains(&cloud.pos.1));
		}
	}
}
