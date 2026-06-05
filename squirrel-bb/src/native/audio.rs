//! `Sound` and `Music` engine tables — no-op stubs.
//!
//! Headless there is no audio backend, but scripts call `Sound.play` (346 sites),
//! `Sound.stopAmbience`, `Music.setTrackList`, etc. throughout. Each is a no-op that
//! accepts any arguments (the calls have several arities/overloads). Real audio is a
//! later, self-contained subsystem.

use squirrel::{Retained, SQVM, SqTable, Variadic};

/// Register the `Sound` and `Music` tables (all methods no-op) into the root.
pub(crate) fn install(vm: &mut SQVM) {
	// One no-op closure shape reused for every method: accept any args, do nothing.
	macro_rules! noop {
		() => {
			vm.make_closure(|Variadic(_args): Variadic<Retained>| {})
		};
	}

	let play = noop!();
	let set_ambience = noop!();
	let stop_ambience = noop!();
	let play_atmosphere = noop!();
	let update = noop!();
	let mut sound = SqTable::new(vm);
	sound.set("play", play);
	sound.set("setAmbience", set_ambience);
	sound.set("stopAmbience", stop_ambience);
	sound.set("playAtmosphere", play_atmosphere);
	sound.set("update", update);
	SqTable::root(vm).set("Sound", sound);

	let set_track_list = noop!();
	let mut music = SqTable::new(vm);
	music.set("setTrackList", set_track_list);
	SqTable::root(vm).set("Music", music);
}
