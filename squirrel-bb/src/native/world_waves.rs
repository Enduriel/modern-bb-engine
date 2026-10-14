//! Waves on the open sea: `World.spawnWaveSprite(brush, pos, drift, lifetime)`
//! (`0x4f1800`, wave node `0x4e4dd0`; disassembled 2026-10-14).
//!
//! A wave of a known brush fades in over a second of real time, drifts by `drift`
//! units per second of frame time while the game's virtual speed isn't zero, and once
//! `lifetime` seconds of virtual time have passed it stops and fades out over a
//! second, then goes. It sits in the effects layer. (Vanilla's `spawnWaves`, the only
//! caller, is never called, so only mods make waves.)

use std::cell::RefCell;
use std::rc::Rc;

/// How long a wave takes to fade in or out, in real seconds.
const FADE: f32 = 1.0;

struct Wave {
	brush: String,
	pos: (f32, f32),
	drift: (f32, f32),
	lifetime: f32,
	/// Clock readings when it appeared.
	born_real: f32,
	born_virtual: f32,
	/// Real time its fade-out began.
	fading_since: Option<f32>,
}

/// A wave, as a renderer reads it.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneWave {
	pub brush: String,
	pub pos: (f32, f32),
	/// 0 to 255.
	pub alpha: f32,
}

/// The waves, with the clocks they run on (advanced by `World.update`).
#[derive(Default)]
pub struct WaveField {
	waves: Vec<Wave>,
	real: f32,
	virtual_time: f32,
}

/// A shared handle.
pub type Waves = Rc<RefCell<WaveField>>;

impl WaveField {
	pub fn spawn(&mut self, brush: &str, pos: (f32, f32), drift: (f32, f32), lifetime: f32) {
		self.waves.push(Wave {
			brush: brush.to_string(),
			pos,
			drift,
			lifetime,
			born_real: self.real,
			born_virtual: self.virtual_time,
			fading_since: None,
		});
	}

	/// One `World.update()`: `delta` real seconds at virtual speed `speed`.
	pub fn advance(&mut self, delta: f32, speed: f32) {
		self.real += delta;
		self.virtual_time += delta * speed;
		let dt = if speed != 0.0 { delta } else { 0.0 };
		let (real, virtual_time) = (self.real, self.virtual_time);
		self.waves.retain_mut(|w| {
			if let Some(since) = w.fading_since {
				return real - since < FADE;
			}
			if virtual_time - w.born_virtual >= w.lifetime {
				w.fading_since = Some(real);
			}
			w.pos.0 += w.drift.0 * dt;
			w.pos.1 += w.drift.1 * dt;
			true
		});
	}

	pub fn scene(&self) -> Vec<SceneWave> {
		self.waves
			.iter()
			.map(|w| {
				let fade_in = ((self.real - w.born_real) / FADE).min(1.0);
				let fade_out = w.fading_since.map_or(1.0, |since| 1.0 - (self.real - since) / FADE);
				SceneWave {
					brush: w.brush.clone(),
					pos: w.pos,
					alpha: (fade_in * fade_out).clamp(0.0, 1.0) * 255.0,
				}
			})
			.collect()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_wave_fades_in_drifts_then_fades_out() {
		let mut f = WaveField::default();
		f.spawn("waves_01", (100.0, 100.0), (-15.0, -15.0), 1.25);
		f.advance(0.5, 1.0);
		let w = &f.scene()[0];
		assert_eq!(w.pos, (92.5, 92.5));
		assert_eq!(w.alpha, 0.5 * 255.0);
		f.advance(0.75, 1.0);
		// 1.25 s of virtual time: it starts fading and moves this last time.
		assert_eq!(f.scene()[0].pos, (81.25, 81.25));
		f.advance(0.5, 1.0);
		assert_eq!(f.scene()[0].pos, (81.25, 81.25), "stopped");
		assert_eq!(f.scene()[0].alpha, 0.5 * 255.0);
		f.advance(0.5, 1.0);
		assert!(f.scene().is_empty());
	}

	#[test]
	fn a_paused_game_holds_waves_still() {
		let mut f = WaveField::default();
		f.spawn("waves_01", (0.0, 0.0), (-15.0, -15.0), 1.25);
		f.advance(5.0, 0.0);
		assert_eq!(f.scene()[0].pos, (0.0, 0.0));
		assert_eq!(f.scene()[0].alpha, 255.0, "fades run on real time");
	}
}
