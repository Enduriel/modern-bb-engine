//! Particle effects (`Tactical.spawnParticleEffect`, `World.spawnParticleEffect`) as
//! game state the renderer draws: blood sprays, dust, fire, smoke. From the original
//! (disassembled 2026-10-14): the native `0x4d1190` reads the effect, the emitter
//! (`0x5521a0`) starts at `0x553450` and runs in its draw, `0x5526f0`.
//!
//! An effect is an emitter at a tile (plus an offset) with brushes and stages:
//! - after `delay` ms it spawns a particle every `1000 / SpawnRate` ms of real time,
//!   catching up on missed ones, while fewer than `Quantity` are alive;
//! - it stops spawning once `LifeTimeQuantity` have spawned (at least `Quantity`; 0 for
//!   endless) or its run (each stage's longest lifetime, summed) is over, and goes once
//!   its last particle has;
//! - a particle takes a random brush and lives through the stages in turn, each for a
//!   random lifetime in its range. Over a stage it eases from its start values to the
//!   next stage's (colour, saturation, scale, speed, force), each picked at random
//!   between the stage's min and max. A stage with `FlickerEffect` (the default) picks
//!   new start values; without, it starts where the previous stage ended. The last
//!   stage holds its values;
//! - it moves by `direction × speed + force` per second from a random spawn offset,
//!   and spins by its torque (degrees per second) from a random rotation.
//!
//! The randomness is an engine generator in the original (not the scripts'), so ours
//! comes from [`EngineRng`].

use squirrel::{ClassBuilder, Number, Retained, SQVM, SqStrongRef, SqTable, Variadic};

use super::EngineRng;

/// One stage of an effect (the scripts' `Stages` entries; missing fields keep the
/// original's defaults, `0x4cde20`).
#[derive(Clone, Debug, PartialEq)]
pub struct Stage {
	/// Seconds.
	pub lifetime: (f32, f32),
	/// ARGB.
	pub color: (u32, u32),
	pub spawn_offset: ((f32, f32), (f32, f32)),
	pub direction: ((f32, f32), (f32, f32)),
	pub force: ((f32, f32), (f32, f32)),
	pub saturation: (f32, f32),
	pub scale: (f32, f32),
	pub velocity: (f32, f32),
	/// Degrees.
	pub rotation: (f32, f32),
	/// Degrees per second.
	pub torque: (f32, f32),
	pub flicker: bool,
}

impl Default for Stage {
	fn default() -> Self {
		Stage {
			lifetime: (1.0, 1.0),
			color: (0xffff_ffff, 0xffff_ffff),
			spawn_offset: ((0.0, 0.0), (0.0, 0.0)),
			direction: ((0.0, 0.0), (0.0, 0.0)),
			force: ((0.0, 0.0), (0.0, 0.0)),
			saturation: (1.0, 1.0),
			scale: (1.0, 1.0),
			velocity: (0.0, 0.0),
			rotation: (0.0, 0.0),
			torque: (0.0, 0.0),
			flicker: true,
		}
	}
}

/// What a script asks for.
#[derive(Clone, Debug, PartialEq)]
pub struct EffectSpec {
	/// Shared with the scenes a renderer reads.
	pub brushes: Vec<std::rc::Rc<str>>,
	/// Where it is: the tile's world position plus the offset.
	pub pos: (f32, f32),
	/// Real-time ms before it starts.
	pub delay: u32,
	pub quantity: u32,
	pub life_time_quantity: u32,
	/// Particles per second.
	pub spawn_rate: u32,
	pub stages: Vec<Stage>,
}

/// The values a particle eases between over a stage.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Values {
	color: u32,
	saturation: f32,
	scale: f32,
	velocity: f32,
	force: (f32, f32),
}

#[derive(Clone, Debug, PartialEq)]
struct Particle {
	brush: usize,
	stage: usize,
	/// Real seconds.
	stage_start: f32,
	lifetime: f32,
	from: Values,
	to: Values,
	pos: (f32, f32),
	direction: (f32, f32),
	rotation: f32,
	torque: f32,
	/// As drawn now.
	shown: Values,
}

#[derive(Clone, Debug, PartialEq)]
struct Emitter {
	id: u64,
	spec: EffectSpec,
	interval: i64,
	last_spawn: i64,
	start: i64,
	/// 0 for endless.
	duration: i64,
	limit: u32,
	spawned: u32,
	running: bool,
	particles: Vec<Particle>,
}

/// A particle as drawn.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneParticle {
	pub brush: std::rc::Rc<str>,
	/// World units, y up.
	pub pos: (f32, f32),
	/// ARGB.
	pub color: u32,
	pub saturation: f32,
	pub scale: f32,
	/// Degrees.
	pub rotation: f32,
}

/// Every running effect of one scene (the battle's or the world's).
pub struct Particles {
	rng: EngineRng,
	emitters: Vec<Emitter>,
	next_id: u64,
}

/// A shared handle (the natives spawn, the scene's update advances, the renderer reads).
pub type ParticlesHandle = std::rc::Rc<std::cell::RefCell<Particles>>;

/// The original's colour blend (`0x4a65f0`): per channel, `a` to `b` by `t`, truncated.
fn blend(a: u32, b: u32, t: f32) -> u32 {
	let (a, b) = (a.to_be_bytes(), b.to_be_bytes());
	let mut out = [0u8; 4];
	for i in 0..4 {
		out[i] = (f32::from(a[i]) * (1.0 - t) + f32::from(b[i]) * t) as u8;
	}
	u32::from_be_bytes(out)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
	a * (1.0 - t) + b * t
}

impl Particles {
	pub fn new(rng: EngineRng) -> Self {
		Particles {
			rng,
			emitters: Vec::new(),
			next_id: 1,
		}
	}

	/// A random float in `0..1`.
	fn unit(&self) -> f32 {
		self.rng.range(0, 0x7fff) as f32 / 32768.0
	}

	/// A value between `min` and `max` (`min` when the range is empty or reversed).
	fn pick(&self, (min, max): (f32, f32)) -> f32 {
		if min < max { min + self.unit() * (max - min) } else { min }
	}

	fn pick_vec(&self, (min, max): ((f32, f32), (f32, f32))) -> (f32, f32) {
		(self.pick((min.0, max.0)), self.pick((min.1, max.1)))
	}

	fn values(&self, stage: &Stage) -> Values {
		Values {
			color: blend(stage.color.1, stage.color.0, self.unit()),
			saturation: self.pick(stage.saturation),
			scale: self.pick(stage.scale),
			velocity: self.pick(stage.velocity),
			force: self.pick_vec(stage.force),
		}
	}

	/// Start an effect at real time `now_ms`; its id, for [`stop`](Self::stop). An
	/// effect without brushes or stages does nothing (the original returns null).
	pub fn spawn(&mut self, spec: EffectSpec, now_ms: i64) -> Option<u64> {
		super::world_scene_changes::note();
		if spec.brushes.is_empty() || spec.stages.is_empty() {
			return None;
		}
		let id = self.next_id;
		self.next_id += 1;
		let interval = i64::from((1000 / spec.spawn_rate.max(1)).max(1));
		let delay = i64::from(spec.delay);
		let run: i64 = spec.stages.iter().map(|s| (s.lifetime.1 * 1000.0).round() as i64).sum();
		let duration = if spec.life_time_quantity == 0 { 0 } else { run + delay };
		let limit = if spec.life_time_quantity == 0 { 0 } else { spec.life_time_quantity.max(spec.quantity) };
		self.emitters.push(Emitter {
			id,
			interval,
			last_spawn: now_ms + delay - interval,
			start: now_ms + delay,
			duration,
			limit,
			spawned: 0,
			running: true,
			particles: Vec::new(),
			spec,
		});
		Some(id)
	}

	/// Stop effect `id` spawning (`die()`); its particles live out their stages.
	pub fn stop(&mut self, id: u64) {
		super::world_scene_changes::note();
		if let Some(e) = self.emitters.iter_mut().find(|e| e.id == id) {
			e.running = false;
			e.limit = 1;
			e.spawned = 1;
		}
	}

	fn new_particle(&self, spec: &EffectSpec, now: f32) -> Particle {
		let first = &spec.stages[0];
		let lifetime = self.pick(first.lifetime);
		let from = self.values(first);
		let to = spec.stages.get(1).map_or(from, |next| self.values(next));
		let brush = self.rng.range(0, spec.brushes.len() as i64 - 1) as usize;
		Particle {
			brush,
			stage: 0,
			stage_start: now,
			lifetime,
			from,
			to,
			pos: self.pick_vec(first.spawn_offset),
			direction: self.pick_vec(first.direction),
			rotation: self.pick(first.rotation),
			torque: self.pick(first.torque),
			shown: from,
		}
	}

	/// One frame at real time `now_ms` (and `now` seconds), `dt` seconds after the last.
	pub fn advance(&mut self, now_ms: i64, now: f32, dt: f32) {
		// They move and fade on real time.
		if !self.emitters.is_empty() {
			super::world_scene_changes::note();
		}
		let mut emitters = std::mem::take(&mut self.emitters);
		for e in &mut emitters {
			let over = e.duration != 0 && now_ms > e.start && now_ms - e.start >= e.duration;
			if over || (e.limit != 0 && e.spawned >= e.limit) {
				e.running = false;
			}
			// Each particle through its stages.
			let mut alive = Vec::with_capacity(e.particles.len());
			for mut p in std::mem::take(&mut e.particles) {
				let mut t = if p.lifetime > 0.0 { (now - p.stage_start) / p.lifetime } else { 1.0 };
				if t >= 1.0 {
					p.stage += 1;
					let Some(stage) = e.spec.stages.get(p.stage) else { continue };
					p.stage_start = now;
					p.lifetime = self.pick(stage.lifetime);
					p.from = if stage.flicker { self.values(stage) } else { p.to };
					if let Some(next) = e.spec.stages.get(p.stage + 1) {
						p.to = self.values(next);
					}
					t = 0.0;
				}
				let last = p.stage + 1 >= e.spec.stages.len();
				let (velocity, force) = if last {
					(p.from.velocity, p.from.force)
				} else {
					p.shown = Values {
						color: blend(p.from.color, p.to.color, t),
						saturation: lerp(p.from.saturation, p.to.saturation, t),
						scale: lerp(p.from.scale, p.to.scale, t),
						velocity: 0.0,
						force: (0.0, 0.0),
					};
					(
						lerp(p.from.velocity, p.to.velocity, t),
						(lerp(p.from.force.0, p.to.force.0, t), lerp(p.from.force.1, p.to.force.1, t)),
					)
				};
				p.pos.0 += (p.direction.0 * velocity + force.0) * dt;
				p.pos.1 += (p.direction.1 * velocity + force.1) * dt;
				p.rotation += p.torque * dt;
				alive.push(p);
			}
			e.particles = alive;
			// New ones, one per interval since the last, up to the quantity alive.
			let room = (e.spec.quantity as usize).saturating_sub(e.particles.len()) as i64;
			if e.running && room > 0 && now_ms > e.last_spawn {
				let due = (now_ms - e.last_spawn) / e.interval;
				let count = due.min(room);
				if count > 0 {
					e.last_spawn = now_ms - (now_ms - e.last_spawn) % e.interval;
					for _ in 0..count {
						let p = self.new_particle(&e.spec, now);
						e.particles.push(p);
						e.spawned += 1;
					}
				}
			}
		}
		emitters.retain(|e| e.running || !e.particles.is_empty());
		self.emitters = emitters;
	}

	/// Every particle as drawn, emitter by emitter.
	pub fn scene(&self) -> Vec<SceneParticle> {
		let mut out = Vec::new();
		for e in &self.emitters {
			for p in &e.particles {
				out.push(SceneParticle {
					brush: e.spec.brushes[p.brush].clone(),
					pos: (e.spec.pos.0 + p.pos.0, e.spec.pos.1 + p.pos.1),
					color: p.shown.color,
					saturation: p.shown.saturation,
					scale: p.shown.scale,
					rotation: p.rotation,
				});
			}
		}
		out
	}

	/// Remove every effect (a new scene).
	pub fn clear(&mut self) {
		super::world_scene_changes::note();
		self.emitters.clear();
	}
}

/// A `vec2f`'s `X` and `Y` (see [`super::vec::position`]).
fn vec2(vm: &SQVM, v: &SqStrongRef) -> Result<(f32, f32), String> {
	super::vec::position_f32(vm, v, "spawnParticleEffect")
}

/// A stage table as the scripts write it (`Const.Tactical.*Particles[i].Stages[j]`).
fn stage(vm: &SQVM, t: &SqStrongRef) -> Result<Stage, String> {
	let d = Stage::default();
	let num = |k: &str| t.get_member::<Number>(vm, k).map(|n| n.0 as f32);
	let pair = |min: &str, max: &str, def: (f32, f32)| (num(min).unwrap_or(def.0), num(max).unwrap_or(def.1));
	let color = |k: &str, def: u32| {
		t.get_member::<SqStrongRef>(vm, k).and_then(|c| super::color::color_argb(vm, &c)).unwrap_or(def)
	};
	let v = |k: &str, def: (f32, f32)| t.get_member::<SqStrongRef>(vm, k).map_or(Ok(def), |v| vec2(vm, &v));
	let vpair = |min: &str, max: &str, def: ((f32, f32), (f32, f32))| -> Result<_, String> { Ok((v(min, def.0)?, v(max, def.1)?)) };
	Ok(Stage {
		lifetime: pair("LifeTimeMin", "LifeTimeMax", d.lifetime),
		color: (color("ColorMin", d.color.0), color("ColorMax", d.color.1)),
		spawn_offset: vpair("SpawnOffsetMin", "SpawnOffsetMax", d.spawn_offset)?,
		direction: vpair("DirectionMin", "DirectionMax", d.direction)?,
		force: vpair("ForceMin", "ForceMax", d.force)?,
		saturation: pair("SaturationMin", "SaturationMax", d.saturation),
		scale: pair("ScaleMin", "ScaleMax", d.scale),
		velocity: pair("VelocityMin", "VelocityMax", d.velocity),
		rotation: pair("RotationMin", "RotationMax", d.rotation),
		torque: pair("TorqueMin", "TorqueMax", d.torque),
		flicker: t.get_member::<bool>(vm, "FlickerEffect").unwrap_or(d.flicker),
	})
}

/// `spawnParticleEffect(persistent, brushes, tile, delay, quantity, lifeTimeQuantity,
/// spawnRate, stages [, offset])`'s effect, the tile's `Pos` its place (`None` if the
/// arguments don't read; an error for a vector that isn't a `vec2f`).
pub(crate) fn parse_effect(vm: &SQVM, args: &[Retained]) -> Result<Option<EffectSpec>, String> {
	let [_persistent, brushes, tile, delay, quantity, life, rate, stages, rest @ ..] = args else {
		return Ok(None);
	};
	let int = |a: &Retained| a.0.get_value::<Number>(vm).map(|n| n.0.max(0.0) as u32);
	let list = |a: &Retained| a.0.get_value::<squirrel::SqArray>(vm);
	let (Some(brushes), Some(stage_list)) = (list(brushes), list(stages)) else { return Ok(None) };
	let brushes: Vec<std::rc::Rc<str>> = (0..brushes.len()).filter_map(|i| brushes.get::<String>(i)).map(Into::into).collect();
	let stages = (0..stage_list.len())
		.filter_map(|i| stage_list.get::<SqStrongRef>(i))
		.map(|t| stage(vm, &t))
		.collect::<Result<Vec<Stage>, String>>()?;
	let offset = rest.first().map_or(Ok((0.0, 0.0)), |o| vec2(vm, &o.0))?;
	let Some(at) = tile.0.get_member::<SqStrongRef>(vm, "Pos") else { return Ok(None) };
	let at = vec2(vm, &at)?;
	let (Some(delay), Some(quantity), Some(life_time_quantity), Some(spawn_rate)) = (int(delay), int(quantity), int(life), int(rate))
	else {
		return Ok(None);
	};
	Ok(Some(EffectSpec {
		brushes,
		pos: (at.0 + offset.0, at.1 + offset.1),
		delay,
		quantity,
		life_time_quantity,
		spawn_rate,
		stages,
	}))
}

/// `Time.getRealTime()` (ms) and `Time.getRealTimeF()` (s).
fn real_now(vm: &SQVM) -> (i64, f32) {
	let time = SqTable::root(vm).get::<SqStrongRef>("Time");
	let call = |name: &str| {
		time.as_ref()
			.and_then(|t| t.call_method_ret::<Number, _>(vm, name, ()).ok().flatten())
			.map_or(0.0, |n| n.0)
	};
	(call("getRealTime") as i64, call("getRealTimeF") as f32)
}

/// Advance `particles` to now (`Time`'s real clock), `dt` seconds after the last frame.
pub(crate) fn advance_now(vm: &SQVM, particles: &ParticlesHandle, dt: f32) {
	let (ms, s) = real_now(vm);
	particles.borrow_mut().advance(ms, s, dt);
}

/// What a script holds for a spawned effect: `die()` stops it spawning.
struct EffectHandle(ParticlesHandle, Option<u64>);

/// Set `spawnParticleEffect` on root table `table` (`Tactical` or `World`): the
/// original's overload with its per-arity slots (8: no offset, 9: with one). Effects go
/// into `particles`; the script gets a `ParticleSystem`.
pub(crate) fn install_on(vm: &mut SQVM, table: &str, particles: &ParticlesHandle) {
	let class = ClassBuilder::<EffectHandle>::new("ParticleSystem")
		.method("die", |e: &mut EffectHandle| {
			if let Some(id) = e.1 {
				e.0.borrow_mut().stop(id);
			}
		})
		.build(vm);
	let leaf = {
		let particles = particles.clone();
		vm.make_closure(move |vm: &SQVM, Variadic(args): Variadic<Retained>| -> Result<SqStrongRef, String> {
			let id = parse_effect(vm, &args)?.and_then(|spec| {
				let (ms, _) = real_now(vm);
				particles.borrow_mut().spawn(spec, ms)
			});
			Ok(class.instantiate(vm, EffectHandle(particles.clone(), id)))
		})
	};
	let dispatch = super::overload::make_dispatcher(vm, "spawnParticleEffect");
	if let Some(mut t) = SqTable::root(vm).get::<SqTable>(table) {
		t.set(&super::overload::overload_key("spawnParticleEffect", 8), leaf.clone());
		t.set(&super::overload::overload_key("spawnParticleEffect", 9), leaf);
		t.set("spawnParticleEffect", dispatch);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn spray() -> EffectSpec {
		EffectSpec {
			brushes: vec!["blood".into()],
			pos: (100.0, 200.0),
			delay: 0,
			quantity: 10,
			life_time_quantity: 10,
			spawn_rate: 200,
			stages: vec![
				Stage {
					lifetime: (0.3, 0.3),
					velocity: (100.0, 100.0),
					direction: ((0.0, 1.0), (0.0, 1.0)),
					scale: (0.01, 0.01),
					..Stage::default()
				},
				Stage {
					lifetime: (1.0, 1.0),
					scale: (0.5, 0.5),
					..Stage::default()
				},
			],
		}
	}

	#[test]
	fn spawns_one_per_interval_up_to_the_quantity() {
		let mut p = Particles::new(EngineRng::seeded(1));
		p.spawn(spray(), 1000).expect("an effect");
		// 200 a second: one every 5 ms, the first right away.
		p.advance(1000, 1.0, 0.0);
		assert_eq!(p.scene().len(), 1);
		p.advance(1004, 1.004, 0.004);
		assert_eq!(p.scene().len(), 1, "not before the next interval");
		p.advance(1005, 1.005, 0.001);
		assert_eq!(p.scene().len(), 2);
		p.advance(1100, 1.1, 0.095);
		assert_eq!(p.scene().len(), 10, "capped at the quantity");
	}

	#[test]
	fn particles_move_ease_and_end() {
		let mut p = Particles::new(EngineRng::seeded(1));
		p.spawn(EffectSpec { quantity: 1, life_time_quantity: 1, ..spray() }, 0).expect("an effect");
		p.advance(5, 0.005, 0.0);
		let first = p.scene()[0].clone();
		assert_eq!(first.pos, (100.0, 200.0));
		assert_eq!(first.scale, 0.01);
		// Halfway through the first stage: speed and scale halfway to the next stage's
		// (0 and 0.5), and up by that speed for the frame.
		p.advance(155, 0.155, 0.15);
		let mid = p.scene()[0].clone();
		assert!((mid.pos.1 - 207.5).abs() < 0.01, "{mid:?}");
		assert!((mid.scale - 0.255).abs() < 0.01, "{mid:?}");
		// Into the last stage, then past it: gone, and so is the emitter.
		p.advance(400, 0.4, 0.0);
		assert_eq!(p.scene().len(), 1);
		p.advance(1500, 1.5, 0.0);
		assert!(p.scene().is_empty());
		assert!(p.emitters.is_empty());
	}

	#[test]
	fn colours_blend_per_channel_truncated() {
		assert_eq!(blend(0x00ff_ffff, 0xffff_ffff, 0.5), 0x7fff_ffff);
	}
}
