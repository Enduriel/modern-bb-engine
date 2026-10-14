//! A head flying off (`spawnHeadEffect`, a decapitation; the original's `0x4d2680`,
//! the effect `0x4ba780`, its update `0x4bae00`; disassembled 2026-10-14):
//! - it starts 42 units above its tile, moved by the script's offset, each layer at
//!   the script's rotation and spinning `±(70–220)°/s` (the engine's stream picks the
//!   direction, then the speed);
//! - it flies for 0.75 s of virtual time in an arc 125 units high to a random spot by
//!   the tile (`x −5–5`, `y 0–5`, drawn after the spin);
//! - landed, it stops spinning and stays on the ground for the battle, over its
//!   blood pool (laid with the first layer's turn and scale).
//!
//! The scripts get the layers' sprites and colour them; the effect reads them as it
//! draws. Not modelled: the original may land a head on a neighbouring tile (taking a
//! second longer), and it marks the tile's properties `IsHeadSpawned`, which no vanilla
//! script reads.

use squirrel::SqStrongRef;

use crate::native::EngineRng;

/// How long a head flies, in seconds.
const FLIGHT: f32 = 0.75;

/// How far above its tile a head starts.
const START_LIFT: f32 = 42.0;

/// A head.
pub(crate) struct Head {
	pub tile: usize,
	/// The layers' sprites, as the scripts hold them.
	pub layers: Vec<SqStrongRef>,
	pub pool: Option<String>,
	from: (f32, f32),
	/// Where it lands, from its tile.
	pub land: (f32, f32),
	start: f32,
	/// Degrees per second, and the turn it starts at.
	torque: f32,
	rotation: f32,
}

impl Head {
	/// A head flying from `tile` (at `tile_pos`) at virtual time `now`.
	pub fn new(rng: &EngineRng, tile: usize, tile_pos: (f32, f32), layers: Vec<SqStrongRef>, pool: Option<String>, offset: (f32, f32), rotation: f32, now: f32) -> Head {
		let direction = if rng.range(0, 1) == 1 { 1.0 } else { -1.0 };
		let torque = direction * (rng.unit() * 150.0 + 70.0);
		let land_y = rng.unit() * 5.0;
		let land_x = rng.unit() * 10.0 - 5.0;
		Head {
			tile,
			layers,
			pool,
			from: (tile_pos.0 + offset.0, tile_pos.1 + START_LIFT + offset.1),
			land: (land_x, land_y),
			start: now,
			torque,
			rotation,
		}
	}

	/// The share of its flight done at virtual time `now`, at most 1.
	fn progress(&self, now: f32) -> f32 {
		((now - self.start) / FLIGHT).min(1.0)
	}

	/// Whether it has landed by virtual time `now`.
	pub fn landed(&self, now: f32) -> bool {
		self.progress(now) >= 1.0
	}

	/// Where it is at virtual time `now`, its tile at `tile_pos`.
	pub fn pos(&self, now: f32, tile_pos: (f32, f32)) -> (f32, f32) {
		let t = self.progress(now);
		let to = (tile_pos.0 + self.land.0, tile_pos.1 + self.land.1);
		let (dx, dy) = (to.0 - self.from.0, to.1 - self.from.1);
		let arc = 0.25 - (t - 0.5) * (t - 0.5);
		(self.from.0 + dx * t, self.from.1 + dy * t + arc * 100.0 * 5.0)
	}

	/// Its turn at virtual time `now`, in degrees: it stops turning as it lands.
	pub fn rotation(&self, now: f32) -> f32 {
		self.rotation + self.torque * (now - self.start).min(FLIGHT)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn flies_in_an_arc_and_lands() {
		let rng = EngineRng::seeded(3);
		let head = Head::new(&rng, 0, (100.0, 100.0), Vec::new(), None, (-45.0, 30.0), 180.0, 10.0);
		assert_eq!(head.pos(10.0, (100.0, 100.0)), (55.0, 172.0), "lifted and offset");
		let land = head.pos(10.75, (100.0, 100.0));
		assert_eq!(land, (100.0 + head.land.0, 100.0 + head.land.1));
		assert!(head.landed(10.75) && !head.landed(10.7));
		assert_eq!(head.rotation(11.0), head.rotation(12.0), "no spin once landed");
		assert!(head.torque.abs() >= 70.0 && head.torque.abs() <= 220.0);
	}
}
