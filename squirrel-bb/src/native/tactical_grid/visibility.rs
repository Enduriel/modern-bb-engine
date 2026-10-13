//! Who sees which battle tiles, as the original computes it (probed 2026-10-13 and
//! disassembled).
//!
//! Each tile keeps a mask of the factions that see it, the player's discovery and fog
//! flags, and the stamp of the last `updateVisibility` that saw it (`IsVisibleForEntity`,
//! `isVisibleToEntity`). `updateVisibility(tile, vision, faction)` marks its own tile and
//! walks rays out from it ([`walk`](super::walk)). A tile is seen within `vision`, plus
//! the viewer's height advantage, at most 3 more; hiding terrain beyond the adjacent ring
//! stays unseen unless the faction already sees it. A tile two or more levels above the
//! viewer, or one holding a sight-blocking entity no more than one level below, is seen
//! but hides what lies behind it. Seeing a tile discovers it for the player, and the
//! player discovers the units standing there; other factions report each actor they see
//! (`onActorSighted`).

use super::TacticalGridState;
use super::walk::{self, BLOCKS, GO_ON, STOP};
use super::zone::faction_bit;

/// The player's faction.
pub(crate) const PLAYER: i64 = 1;

/// What the walk needs to know about whoever stands on a tile.
#[derive(Clone, Copy)]
pub(crate) struct Occupant {
	pub identity: i64,
	pub is_actor: bool,
	pub blocks_sight: bool,
	pub discovered: bool,
}

/// What a vision update caused, in the order the original fires it.
#[derive(Default)]
pub(crate) struct Sightings {
	/// Units the player discovered (they run `onDiscovered`).
	pub discovered: Vec<i64>,
	/// Actors a non-player viewer saw (it runs `onActorSighted(actor)` for each).
	pub sighted: Vec<i64>,
}

impl TacticalGridState {
	/// Mark tile `id` seen by `faction` (with the player's discovery and fog flags).
	fn mark_seen(&mut self, id: usize, faction: i64) {
		let t = &mut self.tiles[id];
		t.vis_mask |= faction_bit(faction);
		if faction == PLAYER {
			t.fogged = false;
			t.is_discovered = true;
		}
	}

	/// `tile.addVisibilityForFaction(f)`.
	pub(crate) fn add_visibility(&mut self, id: usize, faction: i64) {
		self.mark_seen(id, faction);
	}

	/// `Tactical.clearVisibility()`: nobody sees anything; every tile is fogged.
	pub(crate) fn clear_visibility(&mut self) {
		for t in &mut self.tiles {
			t.fogged = true;
			t.vis_mask = 0;
		}
	}

	/// `Tactical.fillVisibility(faction, visible)`: every tile seen (or unseen) by
	/// `faction`. Filling for the player discovers every tile and every unit on one.
	pub(crate) fn fill_visibility(&mut self, occupants: &mut [Option<Occupant>], faction: i64, visible: bool) -> Sightings {
		let mut out = Sightings::default();
		for id in 0..self.tiles.len() {
			if visible {
				self.mark_seen(id, faction);
				if faction == PLAYER {
					discover(occupants, id, &mut out);
				}
			} else {
				let t = &mut self.tiles[id];
				t.vis_mask &= !faction_bit(faction);
				if faction == PLAYER {
					t.fogged = true;
				}
			}
		}
		out
	}

	/// `entity.updateVisibility(tile, vision, faction)` for the unit `viewer`.
	pub(crate) fn update_visibility(
		&mut self,
		occupants: &mut [Option<Occupant>],
		from: usize,
		vision: i64,
		faction: i64,
	) -> Sightings {
		self.vis_stamp = self.vis_stamp.wrapping_add(1);
		let stamp = self.vis_stamp;
		self.mark_seen(from, faction);
		self.tiles[from].vis_stamp = stamp;
		let mut out = Sightings::default();
		let from_level = self.tiles[from].level;
		walk::walk_rays(self, from, 0x3f, |g, id| {
			let dist = super::hex_distance(&g.tiles[from], &g.tiles[id]);
			let over = dist - vision;
			if over > 3 {
				return STOP;
			}
			let t = &g.tiles[id];
			let hidden = t.is_hiding_entity && dist > 1 && t.vis_mask & faction_bit(faction) == 0;
			if !hidden {
				let t = &mut g.tiles[id];
				t.vis_mask |= faction_bit(faction);
				if faction == PLAYER {
					t.fogged = false;
				}
				t.vis_stamp = stamp;
				if faction == PLAYER {
					discover(occupants, id, &mut out);
				} else if let Some(o) = occupants[id].filter(|o| o.is_actor) {
					out.sighted.push(o.identity);
				}
			}
			if faction == PLAYER {
				g.tiles[id].is_discovered = true;
			}
			sight_answer(g, occupants, from_level, id, over)
		});
		out
	}

	/// `tile.hasLineOfSightTo(target, range)`: whether rays from `from`, cast through
	/// the three directions facing `to`, reach it.
	pub(crate) fn line_of_sight(&mut self, occupants: &[Option<Occupant>], from: usize, to: usize, range: i64) -> bool {
		if from == to {
			return true;
		}
		let d = super::direction_between(self, from, to);
		let cone = (1u8 << d) | (1u8 << ((d + 5) % 6)) | (1u8 << ((d + 1) % 6));
		let from_level = self.tiles[from].level;
		let mut found = false;
		walk::walk_rays(self, from, cone, |g, id| {
			if found {
				return BLOCKS;
			}
			if id == to {
				found = true;
				return BLOCKS;
			}
			let over = super::hex_distance(&g.tiles[from], &g.tiles[id]) - range;
			if over > 3 {
				return STOP;
			}
			sight_answer(g, occupants, from_level, id, over)
		});
		found
	}
}

/// Whether the view goes on past tile `id`: not beyond the range (stretched by the
/// viewer's height advantage), not past a tile two levels up or a sight-blocker no more
/// than one level down.
fn sight_answer(g: &TacticalGridState, occupants: &[Option<Occupant>], from_level: i64, id: usize, over: i64) -> u32 {
	let level = g.tiles[id].level;
	if over >= from_level - level {
		return STOP;
	}
	let rel = level - from_level;
	if occupants[id].is_some_and(|o| o.blocks_sight) && rel >= -1 {
		return BLOCKS;
	}
	if rel >= 2 {
		return BLOCKS;
	}
	GO_ON
}

/// The player discovers whoever stands on tile `id` (once).
fn discover(occupants: &mut [Option<Occupant>], id: usize, out: &mut Sightings) {
	if let Some(o) = occupants[id].as_mut() {
		if !o.discovered {
			o.discovered = true;
			out.discovered.push(o.identity);
		}
	}
}
