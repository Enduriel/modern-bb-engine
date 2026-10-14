//! The `Settings` engine table + the `ControlSettings` value class; the video mode is
//! [`super::video_mode`]'s.
//!
//! Headless: a software cursor, and the gameplay settings the original reports. The
//! video mode is kept and shared with the host, which applies and keeps it; the sound
//! setters and `load` are stubs. `main_menu_state.onShow` reads
//! `Settings.getVideoMode().Width/Height`; `cursor` reads
//! `getControlSettings().UseHardwareCursor`; the Options screen reads every field and
//! the display modes ([`super::display_modes`]).

use squirrel::{ClassBuilder, Native, Retained, SQVM, SqArray, SqStrongRef, SqTable, Variadic};

use super::display_modes;
use super::video_mode::{self, ModeRef, VideoMode, VideoSettings};

/// Input/control settings — the payload behind a `ControlSettings` instance.
#[derive(Clone, Copy)]
struct ControlSettings {
	use_hardware_cursor: bool,
	use_drag_style_scrolling: bool,
}

/// Register the `Settings` table and its value classes into the root; the video mode
/// they share with the host.
pub(crate) fn install(vm: &mut SQVM) -> VideoSettings {
	let current = VideoSettings::default();
	let video = video_mode::class(vm);
	let controls = ClassBuilder::<ControlSettings>::new("ControlSettings")
		.property(
			"UseHardwareCursor",
			|c| c.use_hardware_cursor,
			|c, b| c.use_hardware_cursor = b,
		)
		.property(
			"UseDragStyleScrolling",
			|c| c.use_drag_style_scrolling,
			|c, b| c.use_drag_style_scrolling = b,
		)
		.build(vm);

	// getVideoMode, setVideoMode, save, saveWithCustomVideoMode: see `video_mode`.
	let video_natives = video_mode::natives(vm, &video, &current);
	// The display modes (minimum width, height and color bits); see `display_modes`.
	let query_resolutions = {
		let video = video.clone();
		vm.make_closure(move |vm: &SQVM, w: i64, h: i64, bits: i64| {
			let mut list = SqArray::new(vm);
			for mode in display_modes::query(w, h, bits) {
				list.push(video.instantiate(vm, ModeRef::Copy(VideoMode::of(mode))));
			}
			list.to_strong()
		})
	};
	let query_index_by_video_mode = vm.make_closure(
		|mode: Native<ModeRef>, w: i64, h: i64, bits: i64| -> i64 {
			display_modes::index_of(mode.0.get().display_mode(), w, h, bits)
		},
	);
	let query_by_index = {
		let video = video.clone();
		vm.make_closure(move |vm: &SQVM, index: i64, w: i64, h: i64, bits: i64| {
			let modes = display_modes::query(w, h, bits);
			usize::try_from(index)
				.ok()
				.and_then(|i| modes.get(i))
				.map(|&mode| video.instantiate(vm, ModeRef::Copy(VideoMode::of(mode))))
		})
	};
	let get_control_settings = vm.make_closure(move |vm: &SQVM| {
		controls.instantiate(
			vm,
			ControlSettings {
				use_hardware_cursor: false,
				use_drag_style_scrolling: true,
			},
		)
	});
	// Gameplay settings: persistent shared tables (the same one each call) so script
	// writes like `getTempGameplaySettings().CameraLocked = false` stick and later reads
	// see them. The fields and values are the ones the original reports (probed
	// 2026-10-13 on a configured install; they are player preferences).
	let gameplay = {
		let mut t = SqTable::new(vm);
		for (name, value) in [
			("AdjustCameraLevel", true),
			("AlwaysFocusCamera", false),
			("AlwaysHideTrees", true),
			("AutoLoot", false),
			("AutoPauseAfterCity", true),
			("DontAutoEndTurns", true),
			("FasterAIMovement", true),
			("FasterPlayerMovement", true),
			("RestoreEquipment", true),
			("ShowOrientationOverlays", true),
			("ShowOverlayStats", true),
		] {
			t.set(name, value);
		}
		t.to_strong()
	};
	let get_gameplay = {
		let g = gameplay.clone();
		vm.make_closure(move || g.clone())
	};
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
	// The sound volumes until the engine's sound service replaces them (`audio`);
	// hardware sound is the original's default.
	let get_sound_volume = vm.make_closure(|Variadic(_a): Variadic<Retained>| -> f64 { 1.0 });
	let is_hardware_sound = vm.make_closure(|| true);
	// The sound setters and `load` are still stubs (the host loads the settings).
	let set_sound_volume = super::stubs::closure(vm, "Settings.setSoundVolume");
	let set_hardware_sound = super::stubs::closure(vm, "Settings.setHardwareSound");
	let load = super::stubs::closure(vm, "Settings.load");

	let mut settings = SqTable::new(vm);
	for (name, f) in video_natives {
		settings.set(name, f);
	}
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

/// A field of `Settings.getTempGameplaySettings()` (e.g. `HideTrees`), false if missing.
pub(crate) fn temp_gameplay_flag(vm: &SQVM, name: &str) -> bool {
	SqTable::root(vm)
		.get::<SqTable>("Settings")
		.and_then(|s| s.get::<SqStrongRef>("getTempGameplaySettings"))
		.and_then(|f| f.call_ret::<SqStrongRef, _>(vm, ()).ok().flatten())
		.and_then(|t| t.get_member::<bool>(vm, name))
		.unwrap_or(false)
}
