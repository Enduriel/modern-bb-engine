//! What rendering decides for the game: which entities the player has seen. The
//! original does this while drawing each world entity (disassembly, 2026-10-13); the
//! drawing itself is the renderer's business, so headless only the decision remains.
//!
//! With fog of war on, an entity is visible to the player when it isn't hidden from
//! them (`isHiddenToPlayer`), or when it shows in the fog (`setVisibleInFogOfWar`) and
//! was discovered before; with fog off everything is visible. An entity becomes
//! visible or hidden at most once per 300 ms of real time. Every entity starts hidden,
//! so the first time it is seen it fades in, is marked discovered (running
//! `onDiscovered` once), and runs `onVisibleToPlayer`.
//!
//! The fades (`0x4e7b4d`; disassembled 2026-10-14) are its sprites': coming into
//! sight, every sprite's alpha drops to 0 at once and fades in over 300 ms of real
//! time; going out of sight, every sprite fades out over 300 ms (and stays, clear).
//! Every colourised sprite, that is: like the entity-level fades they are, they leave
//! a sprite with `setSpriteColorization(name, false)` alone (the slot's `+0x1c`), so a
//! settlement's `lighting`, which its script keeps clear by day, stays clear.
//! Its labels aren't sprites and don't fade: they follow the entity's state.
//!
//! Also `World.render()`, which with fog on uncovers the map around the player's view
//! every frame, and `World.prepareRender()`, which only prepares drawing.
//!
//! Divergence: the original only draws (and so only discovers) entities on screen. The
//! camera follows the player and the vision radius is smaller than the screen, so
//! headless every entity is considered drawn.

use std::cell::RefCell;
use std::rc::Rc;

use squirrel::{Number, SQVM, SqStrongRef, SqTable};

use super::{EntityMachine, ignore_missing, view};

/// How long a fade between hidden and visible takes, in real-time ms.
const FADE_MS: i64 = 300;

/// An entity-level fade, done to each of its colourised sprites (the original's
/// `fadeIn` `0x4a9930`, `fadeOut` `0x4a99c0` and `setAlpha` `0x4a9ac0`, which the
/// scripts can also call on the entity; each skips a slot whose `+0x1c` is off).
#[derive(Clone, Copy, Debug)]
pub(super) enum Fade {
	/// To full alpha over ms.
	In(i64),
	/// To clear over ms (the sprites stay).
	Out(i64),
	/// This alpha at once, ending any fade.
	Alpha(i64),
}

/// Do `fade` to every sprite of entity `idx`, at real time `now` (seconds).
pub(super) fn fade_sprites(m: &Machine, vm: &SQVM, idx: usize, fade: Fade, now: f32) {
	let (class, handles) = {
		let machine = m.borrow();
		let handles: Vec<SqStrongRef> = machine.entities[idx].sprites.iter().map(|(_, h)| h.clone()).collect();
		(machine.sprite_class.clone(), handles)
	};
	let Some(class) = class else { return };
	for handle in handles {
		class.with_payload(vm, &handle, |s| {
			if !s.colorization {
				return;
			}
			match fade {
				Fade::In(ms) => s.color.fade_in(ms, now),
				Fade::Out(ms) => s.color.fade_out(ms, now, false),
				Fade::Alpha(alpha) => s.color.snap_alpha(alpha),
			}
		});
	}
}

/// The render-side state of one entity.
#[derive(Clone, Copy)]
pub(super) struct RenderState {
	/// Currently faded out (the original's `+0x119`; every entity starts hidden).
	hidden: bool,
	/// Real time (ms) before which it can't change between hidden and visible.
	next_change: i64,
}

impl RenderState {
	/// Not faded out.
	pub(super) fn shown(&self) -> bool {
		!self.hidden
	}
}

impl Default for RenderState {
	fn default() -> Self {
		RenderState {
			hidden: true,
			next_change: 0,
		}
	}
}

type Machine = Rc<RefCell<EntityMachine>>;

/// `World.isUsingFogOfWar()`.
fn fog_on(vm: &SQVM) -> Result<bool, String> {
	let world = SqTable::root(vm).get::<SqStrongRef>("World").ok_or("World missing")?;
	Ok(world.call_method_ret::<bool, _>(vm, "isUsingFogOfWar", ())?.unwrap_or(true))
}

/// `Time.getRealTime()` (ms).
pub(super) fn real_time_ms(vm: &SQVM) -> Result<i64, String> {
	let time = SqTable::root(vm).get::<SqStrongRef>("Time").ok_or("Time missing")?;
	Ok(time.call_method_ret::<Number, _>(vm, "getRealTime", ())?.map_or(0, |n| n.0 as i64))
}

/// One render pass over the live entities, in id order. No machine borrow is held
/// while scripts run.
pub(super) fn run_pass(m: &Machine, vm: &SQVM) -> Result<(), String> {
	if !super::scene_shown(vm)? {
		return Ok(());
	}
	let fog = fog_on(vm)?;
	let now = real_time_ms(vm)?;
	let order: Vec<usize> = {
		let machine = m.borrow();
		let mut ids: Vec<usize> = (0..machine.entities.len()).filter(|&i| machine.entities[i].alive).collect();
		ids.sort_by_key(|&i| machine.entities[i].id);
		ids
	};
	// The pass holds slots: none is taken over until it's done.
	m.borrow_mut().walking += 1;
	let result = sight_pass(m, vm, order, fog, now);
	m.borrow_mut().walking -= 1;
	result?;
	draw_sprites(m, vm, now as f32 * 0.001);
	Ok(())
}

/// Each entity of `order` (by id) comes into sight or goes out of it, and a dying one
/// whose fade is over dies.
fn sight_pass(m: &Machine, vm: &SQVM, order: Vec<usize>, fog: bool, now: i64) -> Result<(), String> {
	for idx in order {
		if super::reap(m, vm, idx, now)? {
			continue;
		}
		// Decide under the borrow; fade and run the callbacks without it.
		let discovered_now = {
			let mut machine = m.borrow_mut();
			let player_view = machine.player_view;
			let rec = &mut machine.entities[idx];
			if !rec.alive || !rec.visible {
				continue;
			}
			let seen = !fog
				|| !view::hidden_to_player(rec, &player_view)
				|| (rec.visible_in_fog_of_war && rec.discovered);
			if rec.render.next_change >= now {
				continue;
			}
			match (seen, rec.render.hidden) {
				(false, false) => {
					rec.render.hidden = true;
					rec.render.next_change = now + FADE_MS;
					None
				}
				(true, true) => {
					let first = !rec.discovered;
					rec.discovered = true;
					rec.render.hidden = false;
					rec.render.next_change = now + FADE_MS;
					rec.entity.clone().map(|e| (e, first))
				}
				_ => continue,
			}
		};
		let seconds = now as f32 * 0.001;
		let Some((entity, first)) = discovered_now else {
			// Out of sight (a live entity always has its object).
			fade_sprites(m, vm, idx, Fade::Out(FADE_MS), seconds);
			continue;
		};
		fade_sprites(m, vm, idx, Fade::Alpha(0), seconds);
		fade_sprites(m, vm, idx, Fade::In(FADE_MS), seconds);
		{
			if first {
				ignore_missing(entity.call_method(vm, "onDiscovered", ()))?;
			}
			ignore_missing(entity.call_method(vm, "onVisibleToPlayer", ()))?;
		}
	}
	Ok(())
}

/// Draw the entities' sprites at real time `now` (seconds): what a draw does to them is
/// finish their fades (see `SpriteColor`), which may hide them. The original draws an
/// entity's sprites whether it's in sight or not (out of sight they're clear).
fn draw_sprites(m: &Machine, vm: &SQVM, now: f32) {
	let machine = m.borrow();
	let Some(class) = machine.sprite_class.clone() else { return };
	for rec in machine.entities.iter().filter(|r| r.alive && r.visible) {
		for (_, handle) in rec.sprites.iter() {
			class.with_payload(vm, handle, |s| {
				if s.visible && s.color.draw(now).hide {
					s.visible = false;
				}
			});
		}
	}
}

/// `World.prepareRender()` and `World.render()`.
pub(super) fn world_methods(m: &Machine, vm: &mut SQVM) -> Vec<(String, SqStrongRef)> {
	let prepare_render = crate::native::stubs::closure(vm, "World.prepareRender");
	let render = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM| -> Result<(), String> {
			if !fog_on(vm)? {
				return Ok(());
			}
			let (pos, radius) = {
				let machine = m.borrow();
				(machine.player_view.pos, machine.player_view.vision_radius)
			};
			let root = SqTable::root(vm);
			let create_vec = root.get::<SqStrongRef>("createVec").ok_or("createVec missing")?;
			let at = create_vec.call_ret::<SqStrongRef, _>(vm, pos)?.ok_or("createVec returned null")?;
			let world = root.get::<SqStrongRef>("World").ok_or("World missing")?;
			world.call_method(vm, "uncoverFogOfWar", (at, f64::from(radius)))
		})
	};
	vec![("prepareRender".to_string(), prepare_render), ("render".to_string(), render)]
}
