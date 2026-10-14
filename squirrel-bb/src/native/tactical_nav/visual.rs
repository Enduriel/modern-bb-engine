//! The path's footsteps, the preview of a move the player is about to make
//! (`navigator.buildVisualisation(entity, settings, ap, fatigue)`, `0x4c8b60`, and
//! `clearVisualisation()`, `0x4c8e10`; disassembled).
//!
//! `buildVisualisation` puts a footstep on every tile of the current path past the
//! unit's own, pointing the way the path enters it (`steps_<dir>`). The footsteps are
//! pale yellow while the unit can afford the way so far, and grey from the first step
//! whose action points or fatigue it can't: the step's terrain cost plus its height
//! difference times the per-level cost, from the navigator's own settings (no fatigue
//! factor, no zones of control). If the last affordable step is a tile someone stands
//! on, where the unit couldn't stop, its footstep is grey too. The footsteps stand on
//! their tiles like units, and `clearVisualisation` takes them away at once.

use super::NavState;
use crate::scene::tactical::ScenePathStep;

/// The footstep brushes, by the direction the path enters the tile.
const BRUSHES: [&str; 6] = ["steps_top", "steps_top_right", "steps_bot_right", "steps_bot", "steps_bot_left", "steps_top_left"];
const AFFORDABLE: u32 = 0xffe0_e5a2;
const UNAFFORDABLE: u32 = 0xff77_7777;

/// A footstep.
#[derive(Clone, Copy)]
pub(crate) struct Marker {
	tile: usize,
	dir: usize,
	affordable: bool,
}

/// `buildVisualisation` for the unit standing on `start`, with `ap` action points and
/// `fatigue` to spend.
pub(super) fn build(st: &mut NavState, start: usize, ap: i64, fatigue: i64) {
	st.markers.clear();
	let settings = st.settings.borrow().clone();
	let g = st.grid.borrow();
	let cost = |costs: &[i32; 16], t: usize| i64::from(*usize::try_from(g.tiles[t].type_).ok().and_then(|i| costs.get(i)).unwrap_or(&0));
	let (mut prev, mut ap_used, mut fatigue_used, mut out) = (start, 0, 0, false);
	for step in &st.path {
		let t = step.tile;
		if !out {
			let climb = (g.tiles[prev].level - g.tiles[t].level).abs();
			ap_used += cost(&settings.ap_costs, t) + climb * i64::from(settings.ap_per_level);
			fatigue_used += cost(&settings.fatigue_costs, t) + climb * i64::from(settings.fatigue_per_level);
			if ap_used > ap || fatigue_used > fatigue {
				out = true;
				if g.tiles[prev].occupant.is_some() {
					if let Some(last) = st.markers.last_mut() {
						last.affordable = false;
					}
				}
			}
		}
		st.markers.push(Marker {
			tile: t,
			dir: usize::try_from(step.dir).unwrap_or(usize::MAX),
			affordable: !out,
		});
		prev = t;
	}
}

/// The footsteps as drawn now (none on a map they don't fit).
pub(super) fn scene(st: &NavState) -> Vec<ScenePathStep> {
	let g = st.grid.borrow();
	st.markers
		.iter()
		.filter_map(|m| {
			Some(ScenePathStep {
				tile: m.tile,
				pos: g.tiles.get(m.tile)?.world_pos(),
				brush: BRUSHES.get(m.dir)?.to_string(),
				color: if m.affordable { AFFORDABLE } else { UNAFFORDABLE },
			})
		})
		.collect()
}
