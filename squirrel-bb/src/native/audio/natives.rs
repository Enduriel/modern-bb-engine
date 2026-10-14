//! The `Sound` and `Music` tables and `Settings`' sound volumes, over the
//! [`SoundManager`] (bindings `0x4aeeff`–`0x4af08d`; disassembled 2026-10-14).
//!
//! - `Sound.play(file[, volume[, pos[, pitch]]])`, four Sqrat overloads (Modern Hooks
//!   wraps each arity's slot): an effect, two-dimensional unless `pos` is a world
//!   position other than `(0, 0)`; nothing plays, and nothing loads, while the effects'
//!   volume (with the master's) is 0. The one-argument form keeps its own volume and
//!   speed at 1. No random draws.
//! - `Sound.setAmbience`, `stopAmbience`, `setAmbienceVolume(layer, volume)` (a
//!   layer's volume for its next sounds): see [`ambience`](super::ambience).
//!   `Sound.update()` does nothing (the original's binding is an empty method; volume
//!   changes apply in `Settings.setSoundVolume`).
//! - `Music.setTrackList(tracks, crossFade[, keepPlaying])` and `Music.isPlaying()`:
//!   see [`music`](super::music).
//! - `Settings.getSoundVolume(channel)` / `setSoundVolume(channel, v)` (`0x500b00`,
//!   `0x500b30`): channel -1 is the master; the music's volume is stored halved, so it
//!   reads back as set. A change re-applies every sound's volume. `isHardwareSound`
//!   is the original's default, true.
//!
//! The original has no `Sound.playAtmosphere` (one vanilla tutorial scenario calls it,
//! and would fail there too).

use std::cell::RefCell;
use std::rc::Rc;

use squirrel::{Number, Retained, SQVM, SqArray, SqStrongRef, SqTable};

use super::ambience::{Entry, layer_index};
use super::manager::{EFFECTS, SoundManager};
use crate::native::overload::{make_dispatcher, overload_key};

type Manager = Rc<RefCell<SoundManager>>;

/// A world position vector's `X`, `Y`.
fn position(vm: &SQVM, v: &Retained) -> (f32, f32) {
	let c = |k: &str| v.0.get_member::<Number>(vm, k).map_or(0.0, |n| n.0 as f32);
	(c("X"), c("Y"))
}

/// `Sound.play` with every argument (`0x4aed40`).
fn play(m: &mut SoundManager, file: &str, volume: f32, at: (f32, f32), pitch: f32) {
	if m.audible(EFFECTS) == 0.0 {
		return;
	}
	let at = (at.0 != 0.0 || at.1 != 0.0).then_some(at);
	let Some(id) = m.start(file, at, EFFECTS, true) else { return };
	let audible = m.audible(EFFECTS) * 1.0;
	m.set_out_volume(id, audible);
	m.set_volume(id, volume);
	m.set_speed(id, pitch);
	m.play(id);
}

/// One ambience entry (a `Const.SoundAmbience` table): `File`, `Volume` and `Pitch`
/// always read; the flags only when present.
fn entry(vm: &SQVM, e: &SqStrongRef) -> Option<Entry> {
	let file = e.get_member::<String>(vm, "File")?;
	let number = |k: &str| e.get_member::<Number>(vm, k).map_or(0.0, |n| n.0 as f32);
	let mut entry = Entry::new(format!("sounds/{file}"), number("Volume"), number("Pitch"));
	let flag = |k: &str, default: bool| e.get_member::<bool>(vm, k).unwrap_or(default);
	entry.pan = flag("Pan", entry.pan);
	entry.random_volume = flag("RandomVolume", entry.random_volume);
	entry.random_pitch = flag("RandomPitch", entry.random_pitch);
	Some(entry)
}

fn strings(vm: &SQVM, list: &SqArray) -> Vec<String> {
	let _ = vm;
	(0..list.len()).filter_map(|i| list.get::<String>(i)).collect()
}

/// Register `Sound` and `Music`, and `Settings`' sound volumes (after `Settings`).
pub(crate) fn install(vm: &mut SQVM, m: &Manager) {
	let mut sound = Vec::new();
	{
		let m1 = m.clone();
		sound.push((
			overload_key("play", 1),
			vm.make_closure(move |file: String| {
				let mut m = m1.borrow_mut();
				if m.audible(EFFECTS) == 0.0 {
					return;
				}
				let Some(id) = m.start(&file, None, EFFECTS, true) else { return };
				let volume = m.audible(EFFECTS) * 1.0;
				m.set_out_volume(id, volume);
				m.play(id);
			}),
		));
		let m2 = m.clone();
		sound.push((
			overload_key("play", 2),
			vm.make_closure(move |file: String, volume: Number| play(&mut m2.borrow_mut(), &file, volume.0 as f32, (0.0, 0.0), 1.0)),
		));
		let m3 = m.clone();
		sound.push((
			overload_key("play", 3),
			vm.make_closure(move |vm: &SQVM, file: String, volume: Number, at: Retained| {
				play(&mut m3.borrow_mut(), &file, volume.0 as f32, position(vm, &at), 1.0)
			}),
		));
		let m4 = m.clone();
		sound.push((
			overload_key("play", 4),
			vm.make_closure(move |vm: &SQVM, file: String, volume: Number, at: Retained, pitch: Number| {
				play(&mut m4.borrow_mut(), &file, volume.0 as f32, position(vm, &at), pitch.0 as f32)
			}),
		));
	}
	sound.push(("play".to_string(), make_dispatcher(vm, "play")));
	let set_ambience = {
		let m = m.clone();
		vm.make_closure(move |vm: &SQVM, layer: Number, entries: SqArray, volume: Number, min_delay: Number| {
			let list: Vec<Entry> = (0..entries.len())
				.filter_map(|i| entries.get::<SqStrongRef>(i))
				.filter_map(|e| entry(vm, &e))
				.collect();
			let layer = layer_index(layer.0 as i64);
			m.borrow_mut().set_ambience(layer, list, volume.0 as f32, min_delay.0 as i64 as u32);
		})
	};
	let stop_ambience = {
		let m = m.clone();
		vm.make_closure(move || m.borrow_mut().stop_ambience())
	};
	let set_ambience_volume = {
		let m = m.clone();
		vm.make_closure(move |layer: Number, volume: Number| {
			m.borrow_mut().ambience[layer_index(layer.0 as i64)].volume = volume.0 as f32;
		})
	};
	sound.push(("setAmbience".to_string(), set_ambience));
	sound.push(("stopAmbience".to_string(), stop_ambience));
	sound.push(("setAmbienceVolume".to_string(), set_ambience_volume));
	sound.push(("update".to_string(), crate::native::stubs::closure(vm, "Sound.update")));

	let mut music = Vec::new();
	{
		let m2 = m.clone();
		music.push((
			overload_key("setTrackList", 2),
			vm.make_closure(move |vm: &SQVM, tracks: SqArray, cross_fade: Number| {
				m2.borrow_mut().set_track_list(strings(vm, &tracks), cross_fade.0 as i64 as u32, false)
			}),
		));
		let m3 = m.clone();
		music.push((
			overload_key("setTrackList", 3),
			vm.make_closure(move |vm: &SQVM, tracks: SqArray, cross_fade: Number, keep: bool| {
				m3.borrow_mut().set_track_list(strings(vm, &tracks), cross_fade.0 as i64 as u32, keep)
			}),
		));
	}
	music.push(("setTrackList".to_string(), make_dispatcher(vm, "setTrackList")));
	let is_playing = {
		let m = m.clone();
		vm.make_closure(move || m.borrow().music_playing())
	};
	music.push(("isPlaying".to_string(), is_playing));

	for (name, slots) in [("Sound", sound), ("Music", music)] {
		let mut table = SqTable::new(vm);
		for (key, f) in slots {
			table.set(&key, f);
		}
		SqTable::root(vm).set(name, table);
	}

	let get_volume = {
		let m = m.clone();
		vm.make_closure(move |channel: Number| -> f64 {
			let m = m.borrow();
			match channel.0 as i64 {
				-1 => f64::from(m.master),
				0 => f64::from(m.channels[0]) * 2.0,
				c => usize::try_from(c).ok().and_then(|c| m.channels.get(c)).map_or(0.0, |v| f64::from(*v)),
			}
		})
	};
	let set_volume = {
		let m = m.clone();
		vm.make_closure(move |channel: Number, v: Number| {
			let mut m = m.borrow_mut();
			let v = v.0 as f32;
			let slot = match channel.0 as i64 {
				-1 => Some((&mut m.master, v)),
				c => usize::try_from(c).ok().filter(|c| *c < 32).map(|c| {
					let v = if c == 0 { v * 0.5 } else { v };
					(&mut m.channels[c], v)
				}),
			};
			let Some((slot, v)) = slot else { return };
			if *slot != v {
				*slot = v;
				m.reapply();
			}
		})
	};
	let is_hardware_sound = vm.make_closure(|| true);
	if let Some(mut settings) = SqTable::root(vm).get::<SqTable>("Settings") {
		settings.set("getSoundVolume", get_volume);
		settings.set("setSoundVolume", set_volume);
		settings.set("isHardwareSound", is_hardware_sound);
	}
}
