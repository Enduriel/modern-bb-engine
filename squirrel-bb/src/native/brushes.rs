//! The sprite registry as scripts see it: `doesBrushExist(name)`, and what a sprite
//! slot's `setBrush` / `getBrush` do with a name.
//!
//! The registry itself ([`SpriteRegistry`]) is built by the host from the game's
//! `.brush` files and handed over with [`Sprites::set`]. In the original, the brushes
//! load at startup, before any script runs. Until a host sets one, the registry is
//! empty, every brush is reported missing, and sprite slots keep any name they get.
//!
//! `doesBrushExist` follows the original (`0x505820` → `0x504a10`): an empty name is
//! `false`, otherwise it is a lookup of the name's hash among all loaded sprites, so
//! case and whitespace matter. Probed 2026-10-13: `null` and numbers answer `false`
//! rather than raising. Scripts use it to gate optional sprites, e.g. a corpse's
//! `*_dead` decals or a beard's `*_top` layer.
//!
//! A sprite slot, also probed: `setBrush` with an unknown or empty name clears the
//! slot (the original logs "Unknown Brush requested"), so `HasBrush` is false and
//! `getBrush()` returns `null`. With a brush, `getBrush()` returns a new object each
//! call with `Name` and `TextureName` (its atlas, e.g. `gfx/world_weather.png`).
//!
//! Loading hints: a brush set on a sprite and a script's `Tactical.addResource(path)`
//! queue a path the renderer can start loading before it is drawn. A battle
//! sprite's (a unit's, a battle tile's detail, an effect's) go in a queue of their own
//! ([`Sprites::take_battle_hints`]): the company's units keep their sprites on the world
//! map, where no battle draws them, so the host warms those only while a battle map is
//! set up. A path is queued again once taken (the host may have dropped it since).

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

use bb_assets::SpriteRegistry;
use squirrel::{Retained, SQVM, SqStrongRef, SqTable};

use crate::native::prng::Prng;

#[derive(Default)]
struct State {
	registry: RefCell<SpriteRegistry>,
	/// The C runtime's `rand()`, which the sprite class's `vary…` draw from.
	crt: Prng,
	loaded: Cell<bool>,
	/// Paths hinted since the last [`Sprites::take_hints`], in order, each once.
	hints: RefCell<Queue>,
	/// Battle sprites' atlases since the last [`Sprites::take_battle_hints`].
	battle_hints: RefCell<Queue>,
	/// Brush names, each kept once (see [`Sprites::name`]).
	names: RefCell<HashSet<Rc<str>>>,
}

/// Paths in the order they came, each once.
#[derive(Default)]
struct Queue {
	paths: Vec<String>,
	seen: HashSet<String>,
}

impl Queue {
	fn push(&mut self, path: &str) {
		if !path.is_empty() && self.seen.insert(path.to_string()) {
			self.paths.push(path.to_string());
		}
	}

	fn take(&mut self) -> Vec<String> {
		self.seen.clear();
		std::mem::take(&mut self.paths)
	}
}

/// The engine's sprite registry, shared with the natives that read it, and the random
/// stream sprites draw from. It holds no VM handles, so it needs no teardown before the
/// VM closes.
#[derive(Clone, Default)]
pub struct Sprites(Rc<State>);

impl Sprites {
	/// Replace the registry (normally once, after loading the brushes at startup).
	pub fn set(&self, registry: SpriteRegistry) {
		*self.0.registry.borrow_mut() = registry;
		self.0.loaded.set(true);
	}

	/// `brush` as a shared name: the world's tiles and details hold their brush's name,
	/// and some 40,000 of them name a few hundred brushes (one string each instead of
	/// one each). Kept for the session, bounded by the names the scripts use.
	pub(crate) fn name(&self, brush: &str) -> Rc<str> {
		let mut names = self.0.names.borrow_mut();
		if let Some(name) = names.get(brush) {
			return name.clone();
		}
		let name: Rc<str> = brush.into();
		names.insert(name.clone());
		name
	}

	/// The C runtime's `rand()`, which the sprite class's `vary…` draw from.
	pub(crate) fn crt(&self) -> Prng {
		self.0.crt.clone()
	}

	/// Read the registry.
	pub fn with<R>(&self, f: impl FnOnce(&SpriteRegistry) -> R) -> R {
		f(&self.0.registry.borrow())
	}

	/// Queue `path` (an atlas, or any resource a script names) as about to be needed.
	pub fn hint(&self, path: &str) {
		self.0.hints.borrow_mut().push(path);
	}

	/// The paths hinted since the last call, each once, in the order they came.
	pub fn take_hints(&self) -> Vec<String> {
		self.0.hints.borrow_mut().take()
	}

	/// The battle sprites' atlases hinted since the last call (see the module's doc).
	pub fn take_battle_hints(&self) -> Vec<String> {
		self.0.battle_hints.borrow_mut().take()
	}

	/// What a sprite slot keeps after `setBrush(name)`: the name of a loaded brush (and
	/// its atlas is hinted), or `""` for an unknown one. Before brushes are loaded,
	/// every name is kept.
	pub(crate) fn resolve(&self, name: &str) -> String {
		self.resolve_into(name, &self.0.hints)
	}

	/// As [`Sprites::resolve`], for a battle sprite (its atlas in the battle queue).
	pub(crate) fn resolve_battle(&self, name: &str) -> String {
		self.resolve_into(name, &self.0.battle_hints)
	}

	fn resolve_into(&self, name: &str, queue: &RefCell<Queue>) -> String {
		if !self.0.loaded.get() {
			return name.to_string();
		}
		let texture = self.with(|r| r.get(name).map(|(sheet, _)| sheet.sheet.texture.clone()));
		match texture {
			Some(texture) => {
				queue.borrow_mut().push(&texture);
				name.to_string()
			}
			None => String::new(),
		}
	}

	/// The flag bits a sprite has after the original's `setBrush(name)` or `resetBrush()`
	/// (`""`; `0x52bf50`), from `bits` before: 0x10 follows a real brush's flag bit 3 (and
	/// stays as it was without one), and 0x4 ("not animating") is set unless the brush has
	/// more than one frame. A sprite no brush was ever set on has neither.
	pub(crate) fn brush_bits(&self, name: &str, bits: u32) -> u32 {
		let brush = if name.is_empty() {
			None
		} else {
			self.with(|r| r.get(name).map(|(_, s)| (s.flags & 8 != 0, s.frames.len() > 1)))
		};
		match brush {
			Some((bit3, animated)) => (if bit3 { 0x10 } else { 0 }) | (if animated { 0 } else { 4 }),
			None => (bits & 0x10) | 4,
		}
	}

	/// The atlas a brush is cut from (`""` if unknown).
	pub(crate) fn texture_of(&self, name: &str) -> String {
		self.with(|r| r.get(name).map(|(sheet, _)| sheet.sheet.texture.clone()))
			.unwrap_or_default()
	}

	/// What `getBrush()` returns for a slot holding `name`: `null` when empty, else a
	/// fresh `{ Name, TextureName }`.
	pub(crate) fn brush_object(&self, vm: &SQVM, name: &str) -> Option<SqStrongRef> {
		if name.is_empty() {
			return None;
		}
		let mut brush = SqTable::new(vm);
		brush.set("Name", name.to_string());
		brush.set("TextureName", self.texture_of(name));
		Some(brush.to_strong())
	}
}

/// Register `doesBrushExist` over a new, empty registry and return the registry, its
/// sprites drawing from `crt`.
pub(crate) fn install(vm: &mut SQVM, crt: Prng) -> Sprites {
	let sprites = Sprites(Rc::new(State { crt, ..State::default() }));
	let lookup = sprites.clone();
	vm.register("doesBrushExist", move |vm: &SQVM, name: Retained| {
		let name = name.0.get_value::<String>(vm);
		name.is_some_and(|name| lookup.with(|r| r.contains(&name)))
	});
	sprites
}

impl crate::memory_report::ReportMemory for Sprites {
	fn report_memory(&self, report: &mut crate::memory_report::MemoryReport) {
		use crate::memory_report::{set_bytes, vec_bytes};
		let registry = self.0.registry.borrow();
		let sheets = registry.sheets();
		let sprites: usize = sheets.iter().map(|s| s.sheet.sprites.len()).sum();
		let bytes: usize = sheets
			.iter()
			.map(|s| s.path.capacity() + s.sheet.texture.capacity() + vec_bytes(&s.sheet.sprites))
			.sum::<usize>()
			+ sheets.iter().flat_map(|s| s.sheet.sprites.iter()).map(|b| b.name.capacity()).sum::<usize>();
		report.add("sprite registry", "brush sheets", sheets.len(), size_of_val(sheets));
		report.add("sprite registry", "sprites in them", sprites, bytes);
		report.add("sprite registry", "sprites by name", registry.len(), registry.len() * (size_of::<(u64, u64)>() + 1));
		let names = self.0.names.borrow();
		let text: usize = names.iter().map(|n| n.len()).sum();
		report.add("sprite registry", "brush names shared", names.len(), set_bytes(&names) + text);
		let hints = self.0.hints.borrow().paths.len() + self.0.battle_hints.borrow().paths.len();
		report.add("sprite registry", "hints not taken yet", hints, 0);
	}
}

#[cfg(test)]
mod tests {
	use bb_assets::{BrushFrame, BrushSheet, BrushSprite, SheetHeader, name_hash};

	use super::*;

	fn registry() -> SpriteRegistry {
		let sprite = BrushSprite {
			hash: name_hash(b"world_birds_01"),
			name: "world_birds_01".into(),
			material: 0,
			width: 1,
			height: 1,
			offset_x: 0,
			offset_y: 0,
			depth_bias: 0,
			frame_time_ms: 100,
			unknown_pause: [0, 0],
			rotation_speed: 0.0,
			flags: 0,
			pivot: [0.0, 0.0],
			color: [0; 4],
			unknown_scale: 2.0,
			frames: vec![BrushFrame { name: String::new(), uv: [0.0; 4], rect: [0.0; 4] }],
			trailer: [0; 3],
		};
		let header = SheetHeader { width: 1, height: 1, flags: [0; 12], sprite_count: 1, reserved: [0; 2], max_level: 1000 };
		let mut registry = SpriteRegistry::new();
		registry.add_sheet(
			"brushes/world_weather.brush",
			BrushSheet { texture: "gfx/world_weather.png".into(), header, sprites: vec![sprite] },
		);
		registry
	}

	#[test]
	fn before_brushes_load_every_name_is_kept() {
		let sprites = Sprites::default();
		assert_eq!(sprites.resolve("anything"), "anything");
		assert!(sprites.take_hints().is_empty());
	}

	#[test]
	fn a_known_brush_is_kept_and_hints_its_atlas_once() {
		let sprites = Sprites::default();
		sprites.set(registry());
		assert_eq!(sprites.resolve("world_birds_01"), "world_birds_01");
		assert_eq!(sprites.resolve("world_birds_01"), "world_birds_01");
		assert_eq!(sprites.take_hints(), ["gfx/world_weather.png"], "once a take");
		assert!(sprites.take_hints().is_empty());
	}

	#[test]
	fn an_unknown_or_empty_brush_clears_the_slot() {
		let sprites = Sprites::default();
		sprites.set(registry());
		assert_eq!(sprites.resolve("no_such_brush"), "");
		assert_eq!(sprites.resolve(""), "");
		assert_eq!(sprites.resolve("WORLD_BIRDS_01"), "");
		assert!(sprites.take_hints().is_empty());
	}

	#[test]
	fn hints_keep_their_order_and_skip_repeats() {
		let sprites = Sprites::default();
		for path in ["sounds/a.wav", "gfx/b.png", "sounds/a.wav", ""] {
			sprites.hint(path);
		}
		assert_eq!(sprites.take_hints(), ["sounds/a.wav", "gfx/b.png"]);
		sprites.hint("gfx/b.png");
		assert_eq!(sprites.take_hints(), ["gfx/b.png"], "queued again once taken");
	}

	#[test]
	fn a_battle_sprites_atlas_goes_in_the_battle_queue() {
		let sprites = Sprites::default();
		sprites.set(registry());
		assert_eq!(sprites.resolve_battle("world_birds_01"), "world_birds_01");
		assert!(sprites.take_hints().is_empty());
		assert_eq!(sprites.take_battle_hints(), ["gfx/world_weather.png"]);
	}
}
