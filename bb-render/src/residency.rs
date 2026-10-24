//! Which textures stay on the GPU: sprites load on use and are dropped once they're no
//! longer needed. A hint only warms a texture
//! (decoded, waiting on the CPU, see `bb_assets::TextureCache::warm`); a draw uploads
//! it. Then it goes:
//! - when the scene that needed it is discarded: at a battle's end
//!   (`Tactical.clearScene`), what the world map didn't draw (the battle's, and the
//!   portraits', see [`gone_with_battle`]); when the campaign is left
//!   (`World.clearScene`), everything;
//! - over the budget (see `Gpu::texture_budget`), the least recently drawn first, the
//!   world map's last, never one this frame drew.
//!
//! No clock: "recently" counts frames, and the rest happens where the game discards a
//! scene, as the reference cycles are collected (`squirrel_bb::collect`). A dropped
//! texture comes back the next time a draw needs it, decoded on a worker; its sprites
//! skip the frames until then (`FrameStats::waiting`).

use std::collections::HashMap;

/// Who draws a texture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Drawer {
	/// The world map.
	World,
	/// A battle.
	Battle,
	/// A portrait for the UI, the cursor.
	Other,
}

impl Drawer {
	fn bit(self) -> u8 {
		match self {
			Drawer::World => 1,
			Drawer::Battle => 2,
			Drawer::Other => 4,
		}
	}
}

/// Whether a texture (or a page's frame) that `drawers` drew (`Drawer` bits) goes when a
/// battle ends, `before` this frame's draws or not: what the battle drew and the world
/// map didn't, and what only portraits and the cursor drew, unless drawn this frame (the
/// cursor). A portrait is composed once into an image the page keeps, so its sprites
/// aren't needed after; the turn bar's portraits of every unit (battles draw them as
/// `Other` too) stayed until the campaign was left, a few hundred frames a dozen
/// battles in. One drawn again comes back as any dropped texture does.
pub(crate) fn gone_with_battle(drawers: u8, before: bool) -> bool {
	let (world, battle) = (Drawer::World.bit(), Drawer::Battle.bit());
	drawers & world == 0 && (drawers & battle != 0 || before)
}

/// A texture's draws: the last frame, and who ever drew it.
#[derive(Clone, Copy, Debug, Default)]
struct Use {
	frame: u64,
	drawers: u8,
}

/// When each texture was last drawn, and by whom.
pub struct Residency {
	frame: u64,
	textures: HashMap<String, Use>,
	budget: u64,
}

impl Residency {
	/// With `budget` bytes of textures kept before the least recently drawn go.
	pub fn new(budget: u64) -> Residency {
		Residency {
			frame: 0,
			textures: HashMap::new(),
			budget,
		}
	}

	pub fn budget(&self) -> u64 {
		self.budget
	}

	pub fn set_budget(&mut self, budget: u64) {
		self.budget = budget;
	}

	/// The frame now (counted from the first).
	pub fn frame(&self) -> u64 {
		self.frame
	}

	/// A new frame starts.
	pub fn next_frame(&mut self) {
		self.frame += 1;
	}

	/// When `path` was last drawn (the frame) and who ever drew it ("world", "battle",
	/// "other", joined by `+`), for a memory snapshot; `None` if it isn't tracked.
	pub fn last_drawn(&self, path: &str) -> Option<(u64, String)> {
		let entry = self.textures.get(path)?;
		let who: Vec<&str> = [(Drawer::World, "world"), (Drawer::Battle, "battle"), (Drawer::Other, "other")]
			.into_iter()
			.filter(|(d, _)| entry.drawers & d.bit() != 0)
			.map(|(_, name)| name)
			.collect();
		Some((entry.frame, who.join("+")))
	}

	/// `path` was drawn this frame by `drawer`.
	pub fn drawn(&mut self, path: &str, drawer: Drawer) {
		let frame = self.frame;
		let entry = match self.textures.get_mut(path) {
			Some(entry) => entry,
			None => self.textures.entry(path.to_string()).or_default(),
		};
		entry.frame = frame;
		entry.drawers |= drawer.bit();
	}

	/// Forget what's no longer `resident` (path, GPU bytes); one not seen before counts
	/// as drawn this frame.
	fn sync(&mut self, resident: &[(String, u64)]) {
		for (path, _) in resident {
			let frame = self.frame;
			self.textures.entry(path.clone()).or_insert(Use { frame, drawers: 0 });
		}
		self.textures.retain(|path, _| resident.iter().any(|(p, _)| p == path));
	}

	/// Which of the `resident` textures to drop now to be within the budget: the least
	/// recently drawn first, the world map's last, never one this frame drew.
	pub fn over_budget(&mut self, resident: &[(String, u64)]) -> Vec<String> {
		self.sync(resident);
		let mut total: u64 = resident.iter().map(|(_, bytes)| bytes).sum();
		let mut order: Vec<&(String, u64)> = resident
			.iter()
			.filter(|(path, _)| self.textures[path].frame < self.frame)
			.collect();
		order.sort_by_key(|(path, _)| {
			let entry = self.textures[path];
			(entry.drawers & Drawer::World.bit() != 0, entry.frame)
		});
		let mut drop = Vec::new();
		for (path, bytes) in order {
			if total <= self.budget {
				break;
			}
			total -= bytes;
			drop.push(path.clone());
		}
		drop
	}

	/// A battle ended: the `resident` textures the world map never drew (see
	/// [`gone_with_battle`]).
	pub fn battle_discarded(&mut self, resident: &[(String, u64)]) -> Vec<String> {
		self.sync(resident);
		let frame = self.frame;
		self.dropping(|entry| gone_with_battle(entry.drawers, entry.frame < frame))
	}

	/// The campaign was left: every `resident` texture this frame didn't draw.
	pub fn campaign_discarded(&mut self, resident: &[(String, u64)]) -> Vec<String> {
		self.sync(resident);
		let frame = self.frame;
		self.dropping(|entry| entry.frame < frame)
	}

	fn dropping(&mut self, drop: impl Fn(&Use) -> bool) -> Vec<String> {
		let gone: Vec<String> = self.textures.iter().filter(|(_, e)| drop(e)).map(|(p, _)| p.clone()).collect();
		for path in &gone {
			self.textures.remove(path);
		}
		gone
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn resident(paths: &[(&str, u64)]) -> Vec<(String, u64)> {
		paths.iter().map(|(p, b)| (p.to_string(), *b)).collect()
	}

	fn sorted(mut v: Vec<String>) -> Vec<String> {
		v.sort();
		v
	}

	#[test]
	fn a_battles_end_drops_what_the_world_map_didnt_draw() {
		let mut r = Residency::new(u64::MAX);
		r.drawn("gfx/world_tiles.png", Drawer::World);
		r.next_frame();
		r.drawn("gfx/object_0.png", Drawer::Battle);
		r.drawn("gfx/entity_0.png", Drawer::Battle);
		r.drawn("gfx/entity_0.png", Drawer::Other);
		r.drawn("gfx/cursor.png", Drawer::Other);
		r.next_frame();
		r.drawn("gfx/portrait_0.png", Drawer::Other);
		r.drawn("gfx/cursor.png", Drawer::Other);
		let all = resident(&[
			("gfx/world_tiles.png", 32),
			("gfx/object_0.png", 16),
			("gfx/entity_0.png", 64),
			("gfx/cursor.png", 1),
			("gfx/portrait_0.png", 8),
		]);
		assert_eq!(
			sorted(r.battle_discarded(&all)),
			["gfx/entity_0.png", "gfx/object_0.png"],
			"the world's stay, and what was drawn for the UI this frame: the battle's go, its portraits too"
		);
		r.drawn("gfx/world_tiles.png", Drawer::World);
		r.next_frame();
		r.drawn("gfx/cursor.png", Drawer::Other);
		let all = resident(&[("gfx/world_tiles.png", 32), ("gfx/cursor.png", 1), ("gfx/portrait_0.png", 8)]);
		assert_eq!(r.battle_discarded(&all), ["gfx/portrait_0.png"], "a portrait drawn before this frame goes");
		r.drawn("gfx/entity_0.png", Drawer::Other);
		r.next_frame();
		r.drawn("ui.png", Drawer::Other);
		let all = resident(&[("gfx/world_tiles.png", 32), ("gfx/entity_0.png", 64), ("ui.png", 1)]);
		assert_eq!(
			sorted(r.campaign_discarded(&all)),
			["gfx/entity_0.png", "gfx/world_tiles.png"],
			"leaving the campaign: all but this frame's (the cursor)"
		);
	}

	#[test]
	fn over_the_budget_the_least_recently_drawn_go_first() {
		let mut r = Residency::new(100);
		r.drawn("old", Drawer::Battle);
		r.drawn("world", Drawer::World);
		r.next_frame();
		r.drawn("newer", Drawer::Other);
		r.next_frame();
		r.drawn("now", Drawer::Battle);
		let all = resident(&[("old", 50), ("world", 50), ("newer", 50), ("now", 50)]);
		assert_eq!(r.over_budget(&all), ["old", "newer"], "down to 100: the world's last, never this frame's");
		let within = resident(&[("world", 50), ("now", 50)]);
		assert!(r.over_budget(&within).is_empty());
	}
}
