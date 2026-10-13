//! The sprite registry as scripts see it: `doesBrushExist(name)`.
//!
//! The registry itself ([`SpriteRegistry`]) is built by the host from the game's
//! `.brush` files and handed over with [`Sprites::set`]. In the original, the brushes
//! load at startup, before any script runs. Until a host sets one, the registry is
//! empty and every brush is reported missing.
//!
//! `doesBrushExist` follows the original (`0x505820` → `0x504a10`): an empty name is
//! `false`, otherwise it is a lookup of the name's hash among all loaded sprites, so
//! case and whitespace matter. Probed 2026-10-13: `null` and numbers answer `false`
//! rather than raising. Scripts use it to gate optional sprites, e.g. a corpse's
//! `*_dead` decals or a beard's `*_top` layer.

use std::cell::RefCell;
use std::rc::Rc;

use bb_assets::SpriteRegistry;
use squirrel::{Retained, SQVM};

/// The engine's sprite registry, shared with the natives that read it. It holds no
/// VM handles, so it needs no teardown before the VM closes.
#[derive(Clone, Default)]
pub struct Sprites(Rc<RefCell<SpriteRegistry>>);

impl Sprites {
	/// Replace the registry (normally once, after loading the brushes at startup).
	pub fn set(&self, registry: SpriteRegistry) {
		*self.0.borrow_mut() = registry;
	}

	/// Read the registry.
	pub fn with<R>(&self, f: impl FnOnce(&SpriteRegistry) -> R) -> R {
		f(&self.0.borrow())
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
