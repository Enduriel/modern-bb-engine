//! The `Settings` engine table + the `VideoMode` / `ControlSettings` value classes.
//!
//! Headless stub: the value classes carry sensible defaults (1920×1080, software
//! cursor) and the setters / persistence are no-ops. `main_menu_state.onShow` reads
//! `Settings.getVideoMode().Width/Height`; `cursor` reads
//! `getControlSettings().UseHardwareCursor`. The Options panel (resolution
//! enumeration, real persistence) is a later concern — fields are filled in as the
//! menu drive surfaces them.

use squirrel::{ClassBuilder, Retained, SQVM, SqTable, Variadic};

/// Display mode — the payload behind a `VideoMode` instance.
#[derive(Clone, Copy)]
struct VideoMode {
	width: i64,
	height: i64,
	ui_scale: f64,
}

/// Input/control settings — the payload behind a `ControlSettings` instance.
#[derive(Clone, Copy)]
struct ControlSettings {
	use_hardware_cursor: bool,
	use_drag_style_scrolling: bool,
}

/// Register the `Settings` table and its value classes into the root.
pub(crate) fn install(vm: &mut SQVM) {
	let video = ClassBuilder::<VideoMode>::new("VideoMode")
		.property("Width", |v| v.width, |v, x| v.width = x)
		.property("Height", |v| v.height, |v, x| v.height = x)
		.property("UIScale", |v| v.ui_scale, |v, x| v.ui_scale = x)
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

	let get_video_mode = {
		let video = video.clone();
		vm.make_closure(move |vm: &SQVM| {
			video.instantiate(
				vm,
				VideoMode {
					width: 1920,
					height: 1080,
					ui_scale: 1.0,
				},
			)
		})
	};
	let get_control_settings = vm.make_closure(move |vm: &SQVM| {
		controls.instantiate(
			vm,
			ControlSettings {
				use_hardware_cursor: false,
				use_drag_style_scrolling: false,
			},
		)
	});
	// Gameplay settings: persistent shared tables (the same one each call) so script
	// writes like `getTempGameplaySettings().CameraLocked = false` stick and later reads
	// see them. Pre-create the fields scripts assign with `=`; add more as needed.
	let gameplay = {
		let mut t = SqTable::new(vm);
		t.set("RestoreEquipment", true);
		t.to_strong()
	};
	let get_gameplay = {
		let g = gameplay.clone();
		vm.make_closure(move || g.clone())
	};
	let temp_gameplay = {
		let mut t = SqTable::new(vm);
		t.set("CameraLocked", false);
		t.set("ShowTracking", true);
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
	let set_video_mode = noop!();
	let save = noop!();
	let save_with_custom = noop!();

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
	SqTable::root(vm).set("Settings", settings);
}
