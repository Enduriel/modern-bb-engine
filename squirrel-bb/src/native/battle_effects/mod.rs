//! The battle's visual effects (`Tactical.spawn…Effect`), as game state the renderer
//! reads (disassembled 2026-10-14; each kind's rules in its own module):
//! - icons (`spawnIconEffect`, what a skill or a status shows over a tile): they queue
//!   per tile, so a tile's icons show one after another; each shows at its tile plus
//!   the offset, fades in, stays and fades out over the given milliseconds of virtual
//!   time, moving by its movement each second (`0x4d0ea0`, `0x4bbec0`). Unlike every
//!   other effect, an icon's sprite ignores the battle's light (`0x4bbe20` sets its
//!   `IgnoreAmbientColor` bit), so it shows at full brightness at night;
//! - attacks (`spawnAttackEffect`, the slash over a target): the same phases at once,
//!   moving by one movement until a switch time and by another after (`0x4d1010`,
//!   `0x4b8af0`);
//! - [`sprite`] effects, [`projectile`]s, [`pool`]s and flying [`head`]s.
//!
//! Effects are pure functions of the virtual time since they began, so the scene
//! reads them at any frame; finished ones are dropped as it does. The camera is
//! never flipped in vanilla, so its mirroring of effects isn't modelled.

mod head;
mod pool;
mod projectile;
mod sprite;
mod timed;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub(crate) use head::Head;
pub(crate) use projectile::{Projectile, flight_time};
pub(crate) use sprite::SpriteEffect;
pub(crate) use timed::Phases;

use self::pool::Pool;
use super::EngineRng;
use super::prng::Prng;
use crate::scene::tactical::{SceneEffect, ScenePool};

/// An icon or attack effect.
#[derive(Clone, Debug, PartialEq)]
struct Floating {
	brush: String,
	/// Where it starts.
	base: (f32, f32),
	scale: f32,
	/// Virtual seconds it starts at.
	start: f32,
	phases: Phases,
	/// Units per second: the first until `switch` seconds in, the second after.
	moves: [(f32, f32); 2],
	switch: f32,
	/// Drawn without the battle's light (icons, not attacks).
	unlit: bool,
}

impl Floating {
	fn pos(&self, elapsed: f32) -> (f32, f32) {
		let first = elapsed.min(self.switch);
		let then = (elapsed - self.switch).max(0.0);
		(
			self.base.0 + self.moves[0].0 * first + self.moves[1].0 * then,
			self.base.1 + self.moves[0].1 * first + self.moves[1].1 * then,
		)
	}
}

/// An effect's alpha byte times `alpha` (0 to 1) in `color`.
fn with_alpha(color: u32, alpha: f32) -> u32 {
	let a = ((color >> 24) as f32 * alpha) as u32;
	(color & 0x00ff_ffff) | (a.min(255) << 24)
}

/// How a head's layer looks, as the script left its sprite.
pub(crate) struct LayerLook {
	pub brush: String,
	pub color: u32,
	pub saturation: f32,
	pub scale: f32,
	pub flipped: bool,
}

/// The battle's effects.
pub(crate) struct BattleEffects {
	floating: Vec<Floating>,
	heads: Vec<Head>,
	sprites: Vec<SpriteEffect>,
	projectiles: Vec<Projectile>,
	pools: Vec<Pool>,
	/// Per tile, the virtual time its last queued icon ends.
	icon_queue: HashMap<usize, f32>,
	rng: EngineRng,
	/// The C runtime's `rand()`, which pools draw from.
	crt: Prng,
}

pub(crate) type BattleEffectsHandle = Rc<RefCell<BattleEffects>>;

impl BattleEffects {
	pub fn handle(rng: EngineRng, crt: Prng) -> BattleEffectsHandle {
		Rc::new(RefCell::new(BattleEffects {
			floating: Vec::new(),
			heads: Vec::new(),
			sprites: Vec::new(),
			projectiles: Vec::new(),
			pools: Vec::new(),
			icon_queue: HashMap::new(),
			rng,
			crt,
		}))
	}

	/// An icon over `tile` (at `pos`), after the tile's queued ones, at virtual time `now`.
	pub fn spawn_icon(&mut self, brush: String, tile: usize, pos: (f32, f32), scale: f32, phases: Phases, movement: (f32, f32), now: f32) {
		let free = self.icon_queue.get(&tile).copied().unwrap_or(now);
		let start = free.max(now);
		self.icon_queue.insert(tile, start + (phases.fade_in + phases.stay + phases.fade_out) * 0.001);
		self.floating.push(Floating {
			brush,
			base: pos,
			scale,
			start,
			phases,
			moves: [movement, (0.0, 0.0)],
			switch: f32::INFINITY,
			unlit: true,
		});
	}

	/// An attack's effect at `pos` from virtual time `now`, moving by `moves[0]` for
	/// `switch_ms` and by `moves[1]` after.
	pub fn spawn_attack(&mut self, brush: String, pos: (f32, f32), phases: Phases, moves: [(f32, f32); 2], switch_ms: f32, now: f32) {
		self.floating.push(Floating {
			brush,
			base: pos,
			scale: 1.0,
			start: now,
			phases,
			moves,
			switch: switch_ms * 0.001,
			unlit: false,
		});
	}

	pub fn spawn_sprite(&mut self, effect: SpriteEffect) {
		self.sprites.push(effect);
	}

	pub fn spawn_head(&mut self, head: Head) {
		self.heads.push(head);
	}

	/// The engine's stream, for an effect's random looks.
	pub fn rng(&self) -> EngineRng {
		self.rng.clone()
	}

	/// The C runtime's `rand()`, which sprites' `vary…` also draw.
	pub fn crt(&self) -> Prng {
		self.crt.clone()
	}

	pub fn spawn_projectile(&mut self, projectile: Projectile) {
		self.projectiles.push(projectile);
	}

	/// A pool of `brush` on `tile` at virtual time `now`.
	pub fn spawn_pool(&mut self, tile: usize, brush: String, alpha: f32, scale: f32, flags: i64, now: f32) {
		let crt = self.crt.rand_default();
		let pool = Pool::new(&self.rng, crt, tile, brush, alpha, scale, flags, now);
		self.pools.push(pool);
	}

	/// `tile.clear(flags)`'s share of the effects at virtual time `now`: what lies on
	/// `tile` carrying any of `flags` (everything there for `None`). The original's pools
	/// carry the script's flags (`0x4d2657`) and a landed head
	/// `DetailFlag.SeveredHead`; a head still in the air has none.
	pub fn clear_tile(&mut self, tile: usize, flags: Option<i64>, now: f32) {
		let cleared = |carried: i64| flags.is_none_or(|f| carried & f != 0);
		self.pools.retain(|p| !(p.tile == tile && cleared(p.flags)));
		self.heads.retain(|h| !(h.tile == tile && h.landed(now) && cleared(head::SEVERED_HEAD)));
	}

	/// The battle is over: nothing is left.
	pub fn clear(&mut self) {
		self.floating.clear();
		self.heads.clear();
		self.sprites.clear();
		self.projectiles.clear();
		self.pools.clear();
		self.icon_queue.clear();
	}

	/// What shows at virtual time `now`, finished effects dropped: the effects over the
	/// battle, and what lies on its tiles (from `tile_pos`, a tile's position; `look`,
	/// how a head's layer sprite looks now).
	pub fn scene(
		&mut self,
		now: f32,
		tile_pos: impl Fn(usize) -> Option<(f32, f32)>,
		look: impl Fn(&squirrel::SqStrongRef) -> Option<LayerLook>,
	) -> (Vec<SceneEffect>, Vec<ScenePool>) {
		self.floating.retain(|f| !f.phases.over((now - f.start) * 1000.0));
		self.sprites.retain(|s| s.alpha(now).is_some());
		self.projectiles.retain(|p| !p.arrived(now));
		self.icon_queue.retain(|_, end| *end > now);
		let mut effects = Vec::new();
		for f in &self.floating {
			let elapsed = now - f.start;
			let Some(alpha) = f.phases.alpha(elapsed * 1000.0) else { continue };
			effects.push(SceneEffect {
				brush: f.brush.clone(),
				pos: f.pos(elapsed),
				scale: f.scale,
				rotation: 0.0,
				color: with_alpha(0xffff_ffff, alpha),
				saturation: 1.0,
				mirrored: false,
				brush_offset: false,
				unlit: f.unlit,
			});
		}
		for s in &self.sprites {
			let Some(alpha) = s.alpha(now) else { continue };
			effects.push(SceneEffect {
				brush: s.brush.clone(),
				pos: s.pos,
				scale: s.scale(now),
				rotation: 0.0,
				color: with_alpha(s.color, alpha),
				saturation: 1.0,
				mirrored: false,
				brush_offset: false,
				unlit: false,
			});
		}
		for p in &self.projectiles {
			effects.push(SceneEffect {
				brush: p.brush.clone(),
				pos: p.pos(now),
				scale: p.scale,
				rotation: p.rotation(now),
				color: with_alpha(0xffff_ffff, p.alpha(now)),
				saturation: 1.0,
				mirrored: p.flipped,
				brush_offset: true,
				unlit: false,
			});
		}
		// Heads in flight over the battle; landed ones on the ground, below.
		let mut landed = Vec::new();
		for h in &self.heads {
			let Some(at) = tile_pos(h.tile) else { continue };
			let rotation = h.rotation(now);
			let looks: Vec<LayerLook> = h.layers.iter().filter_map(&look).collect();
			if h.landed(now) {
				landed.push((h, at, rotation, looks));
				continue;
			}
			let pos = h.pos(now, at);
			for l in looks {
				effects.push(SceneEffect {
					brush: l.brush,
					pos,
					scale: l.scale,
					rotation,
					color: l.color,
					saturation: l.saturation,
					mirrored: l.flipped,
					brush_offset: true,
					unlit: false,
				});
			}
		}
		let mut pools: Vec<ScenePool> = self
			.pools
			.iter()
			.filter_map(|p| {
				let at = tile_pos(p.tile)?;
				let off = p.offset(now);
				Some(ScenePool {
					tile: p.tile,
					brush: p.brush.clone(),
					pos: (at.0 + off.0, at.1 + off.1),
					scale: p.scale(now),
					rotation: p.rotation,
					color: p.color,
					saturation: 1.0,
					mirrored: p.mirrored,
				})
			})
			.collect();
		// A landed head: its blood pool, turned and sized as its first layer, then the
		// layers where it came to rest.
		for (h, at, rotation, looks) in landed {
			if let (Some(pool), Some(first)) = (&h.pool, looks.first()) {
				pools.push(ScenePool {
					tile: h.tile,
					brush: pool.clone(),
					pos: at,
					scale: first.scale,
					rotation,
					color: 0xffff_ffff,
					saturation: 1.0,
					mirrored: first.flipped,
				});
			}
			for l in looks {
				pools.push(ScenePool {
					tile: h.tile,
					brush: l.brush,
					pos: (at.0 + h.land.0, at.1 + h.land.1),
					scale: l.scale,
					rotation,
					color: l.color,
					saturation: l.saturation,
					mirrored: l.flipped,
				});
			}
		}
		(effects, pools)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn effects() -> BattleEffects {
		Rc::try_unwrap(BattleEffects::handle(EngineRng::seeded(1), Prng::new())).ok().unwrap().into_inner()
	}

	const ICON: Phases = Phases {
		fade_in: 100.0,
		stay: 550.0,
		fade_out: 180.0,
	};

	#[test]
	fn a_tiles_icons_queue() {
		let mut e = effects();
		e.spawn_icon("a".into(), 7, (0.0, 190.0), 0.75, ICON, (0.0, -200.0), 10.0);
		e.spawn_icon("b".into(), 7, (0.0, 190.0), 0.75, ICON, (0.0, -200.0), 10.0);
		e.spawn_icon("c".into(), 8, (0.0, 190.0), 0.75, ICON, (0.0, -200.0), 10.0);
		let (shown, _) = e.scene(10.5, |_| None, |_| None);
		let names: Vec<_> = shown.iter().map(|s| s.brush.as_str()).collect();
		assert_eq!(names, ["a", "c"], "b waits for a");
		assert_eq!(shown[0].pos, (0.0, 90.0), "200 down a second");
		let (shown, _) = e.scene(10.9, |_| None, |_| None);
		let names: Vec<_> = shown.iter().map(|s| s.brush.as_str()).collect();
		assert_eq!(names, ["b"], "a and c are over");
	}

	#[test]
	fn icons_ignore_the_battles_light_and_attacks_take_it() {
		let mut e = effects();
		e.spawn_icon("icon".into(), 7, (0.0, 0.0), 1.0, ICON, (0.0, 0.0), 0.0);
		e.spawn_attack("attack".into(), (0.0, 0.0), ICON, [(0.0, 0.0); 2], 100.0, 0.0);
		let (shown, _) = e.scene(0.2, |_| None, |_| None);
		let unlit: Vec<_> = shown.iter().map(|s| (s.brush.as_str(), s.unlit)).collect();
		assert_eq!(unlit, [("icon", true), ("attack", false)], "0x4bbe20 flags only the icon's sprite");
	}

	#[test]
	fn an_attack_switches_movement() {
		let mut e = effects();
		let p = Phases {
			fade_in: 100.0,
			stay: 300.0,
			fade_out: 100.0,
		};
		e.spawn_attack("uproot".into(), (0.0, 0.0), p, [(0.0, 80.0), (0.0, -80.0)], 200.0, 0.0);
		let (shown, _) = e.scene(0.3, |_| None, |_| None);
		assert!((shown[0].pos.1 - 8.0).abs() < 1e-4, "up 16, down 8: {:?}", shown[0].pos);
	}
}
