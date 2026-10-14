//! What rendering decides for the game: which entities the player has seen. The
//! original does this while drawing each world entity (disassembly, 2026-10-13); the
//! drawing itself is the renderer's business, so headless only the decision remains.
//!
//! With fog of war on, an entity is visible to the player when it isn't hidden from
//! them (`isHiddenToPlayer`), or when it shows in the fog (`setVisibleInFogOfWar`) and
//! was discovered before; with fog off everything is visible. An entity becomes
//! visible or hidden at most once per 300 ms of real time (the fade). Every entity
//! starts hidden, so the first time it is seen it fades in, is marked discovered
//! (running `onDiscovered` once), and runs `onVisibleToPlayer`.
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
	let fog = fog_on(vm)?;
	let now = real_time_ms(vm)?;
	let order: Vec<usize> = {
		let machine = m.borrow();
		let mut ids: Vec<usize> = (0..machine.entities.len()).filter(|&i| machine.entities[i].alive).collect();
		ids.sort_by_key(|&i| machine.entities[i].id);
		ids
	};
	for idx in order {
		if super::reap(m, vm, idx, now)? {
			continue;
		}
		// Decide under the borrow; run the callbacks without it.
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
					continue;
				}
				(true, true) => {
					let first = !rec.discovered;
					rec.discovered = true;
					rec.render.hidden = false;
					rec.render.next_change = now + FADE_MS;
					Some((rec.entity.clone(), first))
				}
				_ => continue,
			}
		};
		if let Some((entity, first)) = discovered_now {
			if first {
				ignore_missing(entity.call_method(vm, "onDiscovered", ()))?;
			}
			ignore_missing(entity.call_method(vm, "onVisibleToPlayer", ()))?;
		}
	}
	draw_sprites(m, vm, now as f32 * 0.001);
	Ok(())
}

/// Draw the shown entities' sprites at real time `now` (seconds): what a draw does to
/// them is finish their fades (see `SpriteColor`), which may hide them.
fn draw_sprites(m: &Machine, vm: &SQVM, now: f32) {
	let machine = m.borrow();
	let Some(class) = machine.sprite_class.clone() else { return };
	for rec in machine.entities.iter().filter(|r| r.alive && r.visible && r.render.shown()) {
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
	let prepare_render = vm.make_closure(|| {});
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
