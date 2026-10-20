//! Counts the changes to what the world map's scene shows besides the map itself: its
//! entities (their place, sight, sprites and labels), clouds, birds, waves, particles,
//! footprints and region names (see [`Engine::world_scene_changes`]). A host that
//! leaves a frame undrawn when it would show what the last did tells that from this
//! count without building the scene to compare it.
//!
//! Noted where those change: every native that sets something on them, every pass that
//! moves them on its own (the sprites' turn and fades, the sight pass, the weather and
//! particles stepping), loads and `World.clearScene`. What moves with time alone is
//! noted for as long as it may: the entities noted as moving (see `moving_sprites`), a
//! wave fading, a particle effect, clouds and birds while the game's speed isn't zero.
//! What the scene reads elsewhere, the clock's light, the camera, the map's extent and
//! fog of war, whether tracks show, the host compares itself (see
//! [`Engine::world_scene_key`]).
//!
//! One count serves the thread (engines are bound to theirs): a change in another
//! engine on the same thread only makes a frame draw that needn't have, never the
//! reverse.
//!
//! [`Engine::world_scene_changes`]: crate::Engine::world_scene_changes
//! [`Engine::world_scene_key`]: crate::Engine::world_scene_key

use std::cell::Cell;

thread_local! {
	static CHANGES: Cell<u64> = const { Cell::new(0) };
}

/// Something the world scene shows changed (or may have).
pub(crate) fn note() {
	CHANGES.with(|c| c.set(c.get().wrapping_add(1)));
}

/// The changes noted so far on this thread.
pub(crate) fn count() -> u64 {
	CHANGES.with(Cell::get)
}
