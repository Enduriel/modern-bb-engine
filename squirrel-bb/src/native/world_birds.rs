//! [`Flock`]: the birds flying over the world map (`World.getWeather().buildBirds(s)`;
//! disassembled: `0x4fece0`, update `0x4fdc30`).
//!
//! `buildBirds` replaces the birds with `rand(MinBirds, MaxBirds)` new ones, each with a
//! brush (`world_birds_02` or `_03`), a position over the map stretched 500 units
//! upwind, a velocity in its range and a sort key of `rand(0, 1000) - 5000`. They all
//! fly one fixed way, `(-1, 0.5)` normalised (west and a little north). Every
//! `World.update()` moves each by its velocity times the frame's real delta while the
//! game's virtual speed isn't zero, as the clouds move. One that leaves the map plus 500
//! units downwind is replaced by a new one on the map's bottom or right edge, moved 500
//! units upwind (edges that suit only that way, as in the original).
//!
//! A bird is drawn white at alpha 0xa8, with a shadow: the same brush at alpha 0x10,
//! 1.25 times as large, 150 units below. `World.clearScene()` takes them away. The
//! randomness is the engine's own, never the scripts'.

use std::cell::RefCell;
use std::rc::Rc;

use super::EngineRng;
use super::world_clouds::MapSpan;
use crate::scene::SceneBird;

const BRUSHES: [&str; 2] = ["world_birds_02", "world_birds_03"];
/// How far past the map birds reach and respawn.
const MARGIN: f32 = 500.0;

/// What a script sets (`createBirdSettings()`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BirdSettings {
	pub min_birds: i64,
	pub max_birds: i64,
	pub min_velocity: f32,
	pub max_velocity: f32,
}

/// The birds over the world map.
pub struct Flock {
	rng: EngineRng,
	settings: BirdSettings,
	span: Option<MapSpan>,
	/// Their way, normalised.
	dir: (f32, f32),
	birds: Vec<SceneBird>,
}

/// A shared handle (the weather natives and the world update both use it).
pub type Birds = Rc<RefCell<Flock>>;

impl Flock {
	pub fn new(rng: EngineRng) -> Self {
		let k = 1.0 / 1.25f64.sqrt() as f32;
		Flock {
			rng,
			settings: BirdSettings::default(),
			span: None,
			dir: (-k, 0.5 * k),
			birds: Vec::new(),
		}
	}

	pub fn birds(&self) -> &[SceneBird] {
		&self.birds
	}

	/// The margins below and above an axis's span for direction component `d`: on the
	/// upwind side only, and only for moving birds.
	fn upwind(&self, d: f32) -> (f32, f32) {
		let margin = if self.settings.max_velocity != 0.0 { MARGIN } else { 0.0 };
		if d >= 0.0 { (margin, 0.0) } else { (0.0, margin) }
	}

	fn bird(&self, pos: Option<(f32, f32)>, span: MapSpan) -> SceneBird {
		let brush = BRUSHES[self.rng.range(0, 1) as usize];
		let pos = pos.unwrap_or_else(|| {
			let (lo_x, hi_x) = self.upwind(self.dir.0);
			let (lo_y, hi_y) = self.upwind(self.dir.1);
			let x = self.rng.between(span.min.0 - lo_x, span.max.0 + hi_x);
			(x, self.rng.between(span.min.1 - lo_y, span.max.1 + hi_y))
		});
		let velocity = self.rng.between(self.settings.min_velocity, self.settings.max_velocity);
		SceneBird {
			brush,
			pos,
			velocity,
			key: self.rng.range(0, 1000) - 5000,
		}
	}

	/// `buildBirds`: replace the birds with a new flock over `span`.
	pub fn build(&mut self, settings: BirdSettings, span: MapSpan) {
		self.settings = settings;
		self.span = Some(span);
		self.birds.clear();
		let (min, max) = (self.settings.min_birds, self.settings.max_birds);
		let n = if max > min { self.rng.range(min, max) } else { min };
		for _ in 0..n.max(0) {
			let bird = self.bird(None, span);
			self.birds.push(bird);
		}
	}

	/// No birds (`World.clearScene()`, or a flock built without a map).
	pub fn clear(&mut self) {
		self.birds.clear();
		self.span = None;
	}

	/// One `World.update()`: `delta` real seconds, or nothing when the game's virtual
	/// speed is zero.
	pub fn advance(&mut self, delta: f32, virtual_speed: f32) {
		let Some(span) = self.span else { return };
		let delta = if virtual_speed != 0.0 { delta } else { 0.0 };
		let (dx, dy) = self.dir;
		let bounds = (span.min.0 - MARGIN, span.min.1 - MARGIN, span.max.0 + MARGIN, span.max.1 + MARGIN);
		let mut gone = 0;
		self.birds.retain_mut(|b| {
			b.pos.0 += dx * b.velocity * delta;
			b.pos.1 += dy * b.velocity * delta;
			let out = (dx <= 0.0 && b.pos.0 < bounds.0)
				|| (dx >= 0.0 && b.pos.0 > bounds.2)
				|| (dy <= 0.0 && b.pos.1 < bounds.1)
				|| (dy >= 0.0 && b.pos.1 > bounds.3);
			gone += usize::from(out);
			!out
		});
		for _ in 0..gone {
			let (lo_x, hi_x) = self.upwind(dx);
			let (lo_y, hi_y) = self.upwind(dy);
			// Along the bottom edge or up the right one, then upwind.
			let (x, y) = if self.rng.range(1, 2) == 1 {
				(self.rng.between(span.min.0 - lo_x, span.max.0 + hi_x), span.min.1)
			} else {
				(span.max.0, self.rng.between(span.min.1 - lo_y, span.max.1 + hi_y))
			};
			let bird = self.bird(Some((x - dx * MARGIN, y - dy * MARGIN)), span);
			self.birds.push(bird);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn span() -> MapSpan {
		MapSpan { min: (0.0, 0.0), max: (10_000.0, 8000.0) }
	}

	#[test]
	fn a_flock_flies_west_and_comes_back_east() {
		let mut f = Flock::new(EngineRng::seeded(7));
		f.build(BirdSettings { min_birds: 12, max_birds: 12, min_velocity: 45.0, max_velocity: 65.0 }, span());
		assert_eq!(f.birds().len(), 12);
		let before: Vec<(f32, f32)> = f.birds().iter().map(|b| b.pos).collect();
		f.advance(1.0, 1.0);
		for (b, p) in f.birds().iter().zip(&before) {
			assert!(b.pos.0 < p.0 && b.pos.1 > p.1, "west and north: {p:?} -> {:?}", b.pos);
		}
		// Paused: nothing moves.
		let now: Vec<(f32, f32)> = f.birds().iter().map(|b| b.pos).collect();
		f.advance(1.0, 0.0);
		assert_eq!(now, f.birds().iter().map(|b| b.pos).collect::<Vec<_>>());
		// Long enough for all to leave: still 12, all respawned east or south.
		for _ in 0..600 {
			f.advance(1.0, 1.0);
		}
		assert_eq!(f.birds().len(), 12);
		f.clear();
		assert!(f.birds().is_empty());
	}
}
