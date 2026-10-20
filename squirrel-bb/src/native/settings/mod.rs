//! The `Settings` engine table + the `ControlSettings` value class; the video mode is
//! [`super::video_mode`]'s, what `save` keeps is [`game`]'s.
//!
//! As the original's (disassembled 2026-10-20, registration `0x5000e9`–`0x500238`):
//! - `getControlSettings()` (`0x500ad0`) hands out the game's one `ControlSettings`
//!   (a global, `0x65f8a8`), wrapped anew each call: a script's writes to it stick
//!   (edge-of-screen scrolling, the hardware cursor) and every later call sees them.
//! - `getGameplaySettings()` / `getTempGameplaySettings()` (`0x500ae0`, `0x500af0`) are
//!   likewise globals; here persistent tables, so writes stick.
//! - `isHardwareSound()` / `setHardwareSound(b)` (`0x500bb0`, `0x500bc0`) read and set
//!   the sound device's flag. It changes nothing audible here; it is kept and saved.
//! - `save()` (`0x500be0`) writes the video mode, the hardware-sound flag, the volumes,
//!   the control and the gameplay settings to the config file;
//!   `saveWithCustomVideoMode(mode)` (`0x500d30`) the same with `mode`. Here the host
//!   keeps them in its own settings file ([`VideoSettings::take_save`]).
//! - `load(videoOnly)` (`0x500e70`) reads the file back: false without one; the hardware
//!   sound flag, then either the video mode (`videoOnly`) or the volumes, control and
//!   gameplay settings. Here it reads back what the host's file holds as the engine
//!   knows it (what the host gave at start, [`crate::Engine::apply_game_settings`], or
//!   last saved).
//!
//! A fresh install starts from [`GameSettings::FRESH_INSTALL`]. `main_menu_state.onShow`
//! reads `Settings.getVideoMode().Width/Height`; `cursor` reads
//! `getControlSettings().UseHardwareCursor`; the Options screen reads every field and
//! the display modes ([`super::display_modes`]).

pub(crate) mod game;

use std::cell::Cell;
use std::rc::Rc;

use squirrel::{ClassBuilder, Native, SQVM, SqArray, SqStrongRef, SqTable, Truthy};

use super::display_modes;
use super::video_mode::{self, ModeRef, VideoMode, VideoSettings};
use game::{GameSettings, SavedSettings};

/// Input/control settings: the game's one, shared by every `ControlSettings` instance.
#[derive(Clone, Copy)]
struct ControlSettings {
	use_hardware_cursor: bool,
	use_drag_style_scrolling: bool,
}

/// Register the `Settings` table and its value classes into the root; the video mode
/// they share with the host.
pub(crate) fn install(vm: &mut SQVM) -> VideoSettings {
	let fresh = GameSettings::FRESH_INSTALL;
	let current = VideoSettings::default();
	let video = video_mode::class(vm);
	let controls = ClassBuilder::<Rc<Cell<ControlSettings>>>::new("ControlSettings")
		.property(
			"UseHardwareCursor",
			|c| c.get().use_hardware_cursor,
			|c, b| c.set(ControlSettings { use_hardware_cursor: b, ..c.get() }),
		)
		.property(
			"UseDragStyleScrolling",
			|c| c.get().use_drag_style_scrolling,
			|c, b| c.set(ControlSettings { use_drag_style_scrolling: b, ..c.get() }),
		)
		.build(vm);

	// getVideoMode and setVideoMode: see `video_mode`.
	let video_natives = video_mode::natives(vm, &video, &current);
	// The display modes (minimum width, height and color bits); see `display_modes`.
	let query_resolutions = {
		let (video, current) = (video.clone(), current.clone());
		vm.make_closure(move |vm: &SQVM, w: i64, h: i64, bits: i64| {
			let mut list = SqArray::new(vm);
			for mode in current.with_modes(|source| display_modes::query(source, w, h, bits)) {
				list.push(video.instantiate(vm, ModeRef::Copy(VideoMode::of(mode))));
			}
			list.to_strong()
		})
	};
	let query_index_by_video_mode = {
		let current = current.clone();
		vm.make_closure(move |mode: Native<ModeRef>, w: i64, h: i64, bits: i64| -> i64 {
			current.with_modes(|source| display_modes::index_of(source, mode.0.get().display_mode(), w, h, bits))
		})
	};
	let query_by_index = {
		let (video, current) = (video.clone(), current.clone());
		vm.make_closure(move |vm: &SQVM, index: i64, w: i64, h: i64, bits: i64| {
			let modes = current.with_modes(|source| display_modes::query(source, w, h, bits));
			usize::try_from(index)
				.ok()
				.and_then(|i| modes.get(i))
				.map(|&mode| video.instantiate(vm, ModeRef::Copy(VideoMode::of(mode))))
		})
	};
	let control_settings = Rc::new(Cell::new(ControlSettings {
		use_hardware_cursor: fresh.hardware_cursor,
		use_drag_style_scrolling: fresh.drag_scrolling,
	}));
	let get_control_settings = vm.make_closure(move |vm: &SQVM| controls.instantiate(vm, control_settings.clone()));
	// Gameplay settings: persistent shared tables (the same one each call) so script
	// writes like `getTempGameplaySettings().CameraLocked = false` stick and later reads
	// see them.
	let gameplay = {
		let mut t = SqTable::new(vm);
		for (name, value) in game::GAMEPLAY.iter().zip(fresh.gameplay) {
			t.set(name, value);
		}
		t.to_strong()
	};
	let get_gameplay = {
		let g = gameplay.clone();
		vm.make_closure(move || g.clone())
	};
	// The executable's data (`0x65f864`), with ShowTracking set at registration
	// (`0x50029f`); none of these are saved.
	let temp_gameplay = {
		let mut t = SqTable::new(vm);
		for (name, value) in [
			("FasterAIMovement", false),
			("FasterPlayerMovement", false),
			("HideTrees", false),
			("HighlightBlockedTiles", false),
			("ShowOverlayStats", false),
			("CameraLocked", false),
			("ShowTracking", true),
		] {
			t.set(name, value);
		}
		t.to_strong()
	};
	let get_temp_gameplay = {
		let t = temp_gameplay.clone();
		vm.make_closure(move || t.clone())
	};
	// The sound volumes until the engine's sound service replaces them (`audio`).
	let get_sound_volume = vm.make_closure(|_channel: i64| -> f64 { 1.0 });
	let set_sound_volume = vm.make_closure(|_channel: i64, _v: f64| {});
	let hardware_sound = Rc::new(Cell::new(fresh.hardware_sound));
	let is_hardware_sound = {
		let h = hardware_sound.clone();
		vm.make_closure(move || h.get())
	};
	let set_hardware_sound = {
		let h = hardware_sound.clone();
		vm.make_closure(move |on: Truthy| h.set(on.0))
	};
	// What's saved: the video mode given, and the rest as the scripts have it now.
	let save = {
		let current = current.clone();
		vm.make_closure(move |vm: &SQVM| {
			let base = current.stored().map_or(GameSettings::FRESH_INSTALL, |s| s.game);
			current.request_save(SavedSettings { video: current.mode(), game: game::read(vm, base) });
		})
	};
	let save_with = {
		let current = current.clone();
		vm.make_closure(move |vm: &SQVM, mode: Native<ModeRef>| {
			let base = current.stored().map_or(GameSettings::FRESH_INSTALL, |s| s.game);
			current.request_save(SavedSettings { video: mode.0.get(), game: game::read(vm, base) });
		})
	};
	let load = {
		let current = current.clone();
		vm.make_closure(move |vm: &SQVM, video_only: Truthy| -> Result<bool, String> {
			let Some(stored) = current.stored() else { return Ok(false) };
			if video_only.0 {
				// The original takes the file's mode as the game's, applied at its next start.
				hardware_sound.set(stored.game.hardware_sound);
				current.set_mode(stored.video);
			} else {
				game::apply(vm, &stored.game)?;
			}
			Ok(true)
		})
	};

	let mut settings = SqTable::new(vm);
	for (name, f) in video_natives {
		settings.set(name, f);
	}
	settings.set("save", save);
	settings.set("saveWithCustomVideoMode", save_with);
	settings.set("getControlSettings", get_control_settings);
	settings.set("getGameplaySettings", get_gameplay);
	settings.set("getTempGameplaySettings", get_temp_gameplay);
	settings.set("getSoundVolume", get_sound_volume);
	settings.set("isHardwareSound", is_hardware_sound);
	settings.set("setSoundVolume", set_sound_volume);
	settings.set("setHardwareSound", set_hardware_sound);
	settings.set("load", load);
	settings.set("queryResolutions", query_resolutions);
	settings.set("queryResolutionIndexByVideoMode", query_index_by_video_mode);
	settings.set("queryResolutionByIndex", query_by_index);
	SqTable::root(vm).set("Settings", settings);
	current
}

/// A field of `Settings.getGameplaySettings()` (e.g. `ShowOrientationOverlays`),
/// `default` if missing.
pub(crate) fn gameplay_flag(vm: &SQVM, name: &str, default: bool) -> bool {
	SqTable::root(vm)
		.get::<SqTable>("Settings")
		.and_then(|s| s.get::<SqStrongRef>("getGameplaySettings"))
		.and_then(|f| f.call_ret::<SqStrongRef, _>(vm, ()).ok().flatten())
		.and_then(|t| t.get_member::<bool>(vm, name))
		.unwrap_or(default)
}

/// A field of `Settings.getTempGameplaySettings()` (e.g. `HideTrees`), false if missing.
pub(crate) fn temp_gameplay_flag(vm: &SQVM, name: &str) -> bool {
	SqTable::root(vm)
		.get::<SqTable>("Settings")
		.and_then(|s| s.get::<SqStrongRef>("getTempGameplaySettings"))
		.and_then(|f| f.call_ret::<SqStrongRef, _>(vm, ()).ok().flatten())
		.and_then(|t| t.get_member::<bool>(vm, name))
		.unwrap_or(false)
}
