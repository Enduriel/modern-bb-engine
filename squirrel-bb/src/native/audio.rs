//! `Sound` and `Music` engine tables, headless.
//!
//! There is no audio backend yet, but scripts call `Sound.play` (346 sites),
//! `Sound.stopAmbience`, `Music.setTrackList`, etc. throughout. The calls do nothing
//! here, with the original's natives and Sqrat overloads (`Sound.play` takes 1 to 4
//! arguments, `Music.setTrackList` 2 or 3; Modern Hooks wraps each arity's slot).
//! `Music.isPlaying()` reads back what the scripts did: true once a track list is set,
//! as in the original, where the music then plays on (probed true on the world map,
//! muted or not).

use std::cell::Cell;
use std::rc::Rc;

use squirrel::{Retained, SQVM, SqArray, SqTable, Variadic};

use super::overload::{make_dispatcher, overload_key};

/// Register the `Sound` and `Music` tables into the root.
pub(crate) fn install(vm: &mut SQVM) {
	// One no-op closure shape: accept any args, do nothing.
	macro_rules! noop {
		() => {
			vm.make_closure(|Variadic(_args): Variadic<Retained>| {})
		};
	}

	let mut sound_slots = Vec::new();
	for arity in 1..=4 {
		sound_slots.push((overload_key("play", arity), noop!()));
	}
	sound_slots.push(("play".to_string(), make_dispatcher(vm, "play")));
	for name in ["setAmbience", "setAmbienceVolume", "stopAmbience", "playAtmosphere", "update"] {
		sound_slots.push((name.to_string(), noop!()));
	}

	// Whether a track list is playing: set by setTrackList (an empty list stops it).
	let playing = Rc::new(Cell::new(false));
	let mut music_slots = Vec::new();
	for arity in [2, 3] {
		let playing = playing.clone();
		let set_track_list = vm.make_closure(move |vm: &SQVM, Variadic(args): Variadic<Retained>| {
			let tracks = args.first().and_then(|a| a.0.get_value::<SqArray>(vm));
			playing.set(tracks.is_some_and(|t| t.len() != 0));
		});
		music_slots.push((overload_key("setTrackList", arity), set_track_list));
	}
	music_slots.push(("setTrackList".to_string(), make_dispatcher(vm, "setTrackList")));
	music_slots.push(("isPlaying".to_string(), vm.make_closure(move || playing.get())));

	for (name, slots) in [("Sound", sound_slots), ("Music", music_slots)] {
		let mut table = SqTable::new(vm);
		for (key, f) in slots {
			table.set(&key, f);
		}
		SqTable::root(vm).set(name, table);
	}
}
