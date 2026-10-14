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
//! queue a path the renderer can start loading before it is drawn.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

use bb_assets::SpriteRegistry;
use squirrel::{Retained, SQVM, SqStrongRef, SqTable};

#[derive(Default)]
struct State {
	registry: RefCell<SpriteRegistry>,
	loaded: Cell<bool>,
	/// Paths hinted since the last [`Sprites::take_hints`], in order.
	hints: RefCell<Vec<String>>,
	/// Every path ever hinted, so each is queued once.
	hinted: RefCell<HashSet<String>>,
}

/// The engine's sprite registry, shared with the natives that read it. It holds no
/// VM handles, so it needs no teardown before the VM closes.
#[derive(Clone, Default)]
pub struct Sprites(Rc<State>);

impl Sprites {
	/// Replace the registry (normally once, after loading the brushes at startup).
	pub fn set(&self, registry: SpriteRegistry) {
		*self.0.registry.borrow_mut() = registry;
		self.0.loaded.set(true);
	}

	/// Read the registry.
	pub fn with<R>(&self, f: impl FnOnce(&SpriteRegistry) -> R) -> R {
		f(&self.0.registry.borrow())
	}

	/// Queue `path` (an atlas, or any resource a script names) as about to be needed.
	pub fn hint(&self, path: &str) {
		if !path.is_empty() && self.0.hinted.borrow_mut().insert(path.to_string()) {
			self.0.hints.borrow_mut().push(path.to_string());
		}
	}

	/// The paths hinted since the last call, each once, in the order they came.
	pub fn take_hints(&self) -> Vec<String> {
		std::mem::take(&mut self.0.hints.borrow_mut())
	}

	/// What a sprite slot keeps after `setBrush(name)`: the name of a loaded brush (and
	/// its atlas is hinted), or `""` for an unknown one. Before brushes are loaded,
	/// every name is kept.
	pub(crate) fn resolve(&self, name: &str) -> String {
		if !self.0.loaded.get() {
			return name.to_string();
		}
		let texture = self.with(|r| r.get(name).map(|(sheet, _)| sheet.sheet.texture.clone()));
		match texture {
			Some(texture) => {
				self.hint(&texture);
				name.to_string()
			}
			None => String::new(),
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

/// Register `doesBrushExist` over a new, empty registry and return the registry.
pub(crate) fn install(vm: &mut SQVM) -> Sprites {
	let sprites = Sprites::default();
	let lookup = sprites.clone();
	vm.register("doesBrushExist", move |vm: &SQVM, name: Retained| {
		let name = name.0.get_value::<String>(vm);
		name.is_some_and(|name| lookup.with(|r| r.contains(&name)))
	});
	sprites
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
			unknown_offset: [0.0, 0.0],
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
		assert_eq!(sprites.take_hints(), ["gfx/world_weather.png"]);
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
		assert!(sprites.take_hints().is_empty());
	}
}
