//! The `Settings` engine table + the `VideoMode` / `ControlSettings` value classes.
//!
//! Headless: the value classes carry sensible defaults (1920×1080 borderless, software
//! cursor). The video mode is kept ([`VideoSettings`], shared with the host); the other
//! setters and persistence are no-ops. `main_menu_state.onShow`
//! reads `Settings.getVideoMode().Width/Height`; `cursor` reads
//! `getControlSettings().UseHardwareCursor`; the Options screen reads every field and
//! the display modes ([`super::display_modes`]). Real persistence is a later concern.

use std::cell::Cell;
use std::rc::Rc;

use squirrel::{ClassBuilder, Native, Retained, SQVM, SqArray, SqStrongRef, SqTable, Variadic};

use super::display_modes::{self, DisplayMode};

/// Display mode — the payload behind a `VideoMode` instance. The fields are the
/// original's (probed); `getVideoMode` returns a fresh instance each call.
#[derive(Clone, Copy)]
struct VideoMode {
	width: i64,
	height: i64,
	color_bits: i64,
	fullscreen: bool,
	borderless: bool,
	vsync: bool,
	depth_of_field: bool,
	ui_scale: f64,
	scene_scale: f64,
	refresh_rate: i64,
}

impl VideoMode {
	/// The default (the setting a fresh install gets).
	const DEFAULT: VideoMode = VideoMode {
		width: 1920,
		height: 1080,
		color_bits: 32,
		fullscreen: false,
		borderless: true,
		vsync: true,
		depth_of_field: false,
		ui_scale: 1.0,
		scene_scale: 1.0,
		refresh_rate: 60,
	};

	/// A display mode as the resolution queries hand it out.
	fn of(mode: DisplayMode) -> VideoMode {
		VideoMode {
			width: mode.width,
			height: mode.height,
			color_bits: mode.color_bits,
			..VideoMode::DEFAULT
		}
	}

	fn display_mode(&self) -> DisplayMode {
		DisplayMode {
			width: self.width,
			height: self.height,
			color_bits: self.color_bits,
		}
	}
}

/// The current video mode, shared by `Settings.getVideoMode`/`setVideoMode` and the
/// host. The host tells it the screen size, and shows the UI at the mode's size divided
/// by its UI scale, as the original does (probed: at UIScale 1.5 on a 2560×1440 screen
/// the page's viewport is 1706×960, at device pixel ratio 1).
#[derive(Clone)]
pub struct VideoSettings(Rc<Cell<VideoMode>>);

impl VideoSettings {
	/// The screen's width and height, and the UI scale.
	pub fn screen(&self) -> (i64, i64, f64) {
		let mode = self.0.get();
		(mode.width, mode.height, mode.ui_scale)
	}

	/// The scene's scale (`SceneScale`).
	pub fn scene_scale(&self) -> f64 {
		self.0.get().scene_scale
	}

	/// The size the UI is laid out at: the screen divided by the UI scale, in whole
	/// pixels (cut down, as the original's 1706 for 2560 / 1.5).
	pub fn ui_size(&self) -> (i64, i64) {
		let (width, height, scale) = self.screen();
		let scale = if scale > 0.0 { scale } else { 1.0 };
		((width as f64 / scale) as i64, (height as f64 / scale) as i64)
	}

	/// Set the screen size, as the host's window has it.
	pub fn set_screen_size(&self, width: i64, height: i64) {
		let mut mode = self.0.get();
		mode.width = width;
		mode.height = height;
		self.0.set(mode);
	}
}

/// Input/control settings — the payload behind a `ControlSettings` instance.
#[derive(Clone, Copy)]
struct ControlSettings {
	use_hardware_cursor: bool,
	use_drag_style_scrolling: bool,
}

/// Register the `Settings` table and its value classes into the root; the video mode
/// they share with the host.
pub(crate) fn install(vm: &mut SQVM) -> VideoSettings {
	let current = VideoSettings(Rc::new(Cell::new(VideoMode::DEFAULT)));
	let video = ClassBuilder::<VideoMode>::new("VideoMode")
		.property("Width", |v| v.width, |v, x| v.width = x)
		.property("Height", |v| v.height, |v, x| v.height = x)
		.property("ColorBits", |v| v.color_bits, |v, x| v.color_bits = x)
		.property("Fullscreen", |v| v.fullscreen, |v, x| v.fullscreen = x)
		.property("Borderless", |v| v.borderless, |v, x| v.borderless = x)
		.property("VSync", |v| v.vsync, |v, x| v.vsync = x)
		.property("DepthOfField", |v| v.depth_of_field, |v, x| v.depth_of_field = x)
		.property("UIScale", |v| v.ui_scale, |v, x| v.ui_scale = x)
		.property("SceneScale", |v| v.scene_scale, |v, x| v.scene_scale = x)
		.property("RefreshRate", |v| v.refresh_rate, |v, x| v.refresh_rate = x)
		.build(vm);
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

	// A fresh instance each call, of the mode last set.
	let get_video_mode = {
		let (video, current) = (video.clone(), current.clone());
		vm.make_closure(move |vm: &SQVM| video.instantiate(vm, current.0.get()))
	};
	// What the Options screen applies (the original also resizes and rescales then).
	let set_video_mode = {
		let current = current.clone();
		vm.make_closure(move |mode: Native<VideoMode>| current.0.set(mode.0))
	};
	// The display modes (minimum width, height and color bits); see `display_modes`.
	let query_resolutions = {
		let video = video.clone();
		vm.make_closure(move |vm: &SQVM, w: i64, h: i64, bits: i64| {
			let mut list = SqArray::new(vm);
			for mode in display_modes::query(w, h, bits) {
				list.push(video.instantiate(vm, VideoMode::of(mode)));
			}
			list.to_strong()
		})
	};
	let query_index_by_video_mode = vm.make_closure(
		|mode: Native<VideoMode>, w: i64, h: i64, bits: i64| -> i64 {
			display_modes::index_of(mode.0.display_mode(), w, h, bits)
		},
	);
	let query_by_index = {
		let video = video.clone();
		vm.make_closure(move |vm: &SQVM, index: i64, w: i64, h: i64, bits: i64| {
			let modes = display_modes::query(w, h, bits);
			usize::try_from(index)
				.ok()
				.and_then(|i| modes.get(i))
				.map(|&mode| video.instantiate(vm, VideoMode::of(mode)))
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
	let get_sound_volume = vm.make_closure(|Variadic(_a): Variadic<Retained>| -> f64 { 1.0 });
	let is_hardware_sound = vm.make_closure(|| false);
	// Setters / persistence: no-ops (accept any args).
	macro_rules! noop {
		() => {
			vm.make_closure(|Variadic(_a): Variadic<Retained>| {})
		};
	}
	let set_sound_volume = noop!();
	let set_hardware_sound = noop!();
	let save = noop!();
	let save_with_custom = noop!();
	let load = noop!();

	let mut settings = SqTable::new(vm);
	settings.set("getVideoMode", get_video_mode);
	settings.set("getControlSettings", get_control_settings);
	settings.set("getGameplaySettings", get_gameplay);
	settings.set("getTempGameplaySettings", get_temp_gameplay);
	settings.set("getSoundVolume", get_sound_volume);
	settings.set("isHardwareSound", is_hardware_sound);
	settings.set("setSoundVolume", set_sound_volume);
	settings.set("setHardwareSound", set_hardware_sound);
	settings.set("setVideoMode", set_video_mode);
	settings.set("save", save);
	settings.set("saveWithCustomVideoMode", save_with_custom);
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
