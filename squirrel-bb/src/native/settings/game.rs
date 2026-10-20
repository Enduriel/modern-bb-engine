//! [`GameSettings`]: what the original keeps in its `config.cfg` beside the video mode,
//! and how the engine reads it from the scripts' `Settings` and gives it back.
//!
//! The original's `Settings.save()` (`0x500be0`; disassembled 2026-10-20) writes a
//! version (15), the video mode, the hardware-sound flag, the master's and the 32
//! channels' volumes, the control settings (2 bytes, `0x65f8a8`) and the gameplay
//! settings (15 bytes, `0x65f854`); the temporary gameplay settings aren't kept. A fresh
//! install starts from the values those globals have in the executable's data, which are
//! [`GameSettings::FRESH_INSTALL`].

use squirrel::{SQVM, SqStrongRef, SqTable};

use super::super::video_mode::VideoMode;

/// `Settings.getGameplaySettings()`'s fields, in the order of the original's struct
/// (byte offsets 0, 1, 2, 4, 5, 6, 7, 8, 9, 10, 11; offset 3 has no script name).
pub const GAMEPLAY: [&str; 11] = [
	"AlwaysFocusCamera",
	"AdjustCameraLevel",
	"ShowOverlayStats",
	"ShowOrientationOverlays",
	"FasterPlayerMovement",
	"FasterAIMovement",
	"AutoLoot",
	"AlwaysHideTrees",
	"DontAutoEndTurns",
	"AutoPauseAfterCity",
	"RestoreEquipment",
];

/// The sound volumes kept, by name, with their `Const.Sound.Channel`: the master, the
/// music, the effects and the ambience (the original keeps all 32 channels; the scripts
/// use these).
pub const VOLUMES: [(&str, i64); 4] = [("master", -1), ("music", 0), ("effects", 1), ("ambience", 2)];

/// The settings the original keeps in its config file besides the video mode.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GameSettings {
	/// The gameplay settings, as [`GAMEPLAY`] names them.
	pub gameplay: [bool; 11],
	/// `getControlSettings().UseHardwareCursor`.
	pub hardware_cursor: bool,
	/// `getControlSettings().UseDragStyleScrolling`.
	pub drag_scrolling: bool,
	/// `isHardwareSound()`.
	pub hardware_sound: bool,
	/// `getSoundVolume` of [`VOLUMES`]' channels.
	pub volumes: [f32; 4],
}

impl GameSettings {
	/// A fresh install's, without a config file: the gameplay and control settings as the
	/// executable's data has them (`0x65f854`: `00 01 00 01 01 00 …`, `0x65f8a8`: `00 01`),
	/// the sound manager's volumes (`0x4adad0`; the music's stored halved, 0.35, reads
	/// 0.7) and hardware sound on.
	pub const FRESH_INSTALL: GameSettings = GameSettings {
		gameplay: [false, true, false, true, false, false, false, false, false, false, false],
		hardware_cursor: false,
		drag_scrolling: true,
		hardware_sound: true,
		volumes: [1.0, 0.7, 0.7, 0.4],
	};

	/// The gameplay setting `name` ([`GAMEPLAY`]).
	pub fn gameplay(&self, name: &str) -> Option<bool> {
		GAMEPLAY.iter().position(|n| *n == name).map(|i| self.gameplay[i])
	}

	/// Set the gameplay setting `name`; false if there's none.
	pub fn set_gameplay(&mut self, name: &str, value: bool) -> bool {
		match GAMEPLAY.iter().position(|n| *n == name) {
			Some(i) => {
				self.gameplay[i] = value;
				true
			}
			None => false,
		}
	}
}

/// Everything `Settings.save()` keeps: the video mode and the rest.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SavedSettings {
	pub video: VideoMode,
	pub game: GameSettings,
}

impl Default for GameSettings {
	fn default() -> Self {
		GameSettings::FRESH_INSTALL
	}
}

/// The scripts' `Settings` table (a mod may have replaced its functions: the engine
/// goes through them, as the scripts do).
fn settings(vm: &SQVM) -> Option<SqStrongRef> {
	SqTable::root(vm).get::<SqStrongRef>("Settings")
}

fn object(vm: &SQVM, settings: &SqStrongRef, getter: &str) -> Option<SqStrongRef> {
	settings.call_method_ret::<SqStrongRef, _>(vm, getter, ()).ok().flatten()
}

/// The settings as the scripts have them now; what's missing reads as `base`'s.
pub(crate) fn read(vm: &SQVM, base: GameSettings) -> GameSettings {
	let mut s = base;
	let Some(settings) = settings(vm) else { return s };
	if let Some(gameplay) = object(vm, &settings, "getGameplaySettings") {
		for (i, name) in GAMEPLAY.iter().enumerate() {
			if let Some(v) = gameplay.get_member::<bool>(vm, name) {
				s.gameplay[i] = v;
			}
		}
	}
	if let Some(controls) = object(vm, &settings, "getControlSettings") {
		s.hardware_cursor = controls.get_member::<bool>(vm, "UseHardwareCursor").unwrap_or(s.hardware_cursor);
		s.drag_scrolling = controls.get_member::<bool>(vm, "UseDragStyleScrolling").unwrap_or(s.drag_scrolling);
	}
	if let Ok(Some(v)) = settings.call_method_ret::<bool, _>(vm, "isHardwareSound", ()) {
		s.hardware_sound = v;
	}
	for (i, (_, channel)) in VOLUMES.iter().enumerate() {
		if let Ok(Some(v)) = settings.call_method_ret::<f64, _>(vm, "getSoundVolume", (*channel,)) {
			s.volumes[i] = v as f32;
		}
	}
	s
}

/// The settings as the scripts have them now.
pub(crate) fn read_now(vm: &SQVM) -> GameSettings {
	read(vm, GameSettings::FRESH_INSTALL)
}

/// Give the scripts' `Settings` these settings, as the original's `load` does.
pub(crate) fn apply(vm: &SQVM, s: &GameSettings) -> Result<(), String> {
	let settings = settings(vm).ok_or("no Settings")?;
	let gameplay = object(vm, &settings, "getGameplaySettings").ok_or("no gameplay settings")?;
	for (name, v) in GAMEPLAY.iter().zip(s.gameplay) {
		gameplay.set_member(vm, name, v)?;
	}
	let controls = object(vm, &settings, "getControlSettings").ok_or("no control settings")?;
	controls.set_member(vm, "UseHardwareCursor", s.hardware_cursor)?;
	controls.set_member(vm, "UseDragStyleScrolling", s.drag_scrolling)?;
	settings.call_method(vm, "setHardwareSound", (s.hardware_sound,))?;
	for ((_, channel), v) in VOLUMES.iter().zip(s.volumes) {
		settings.call_method(vm, "setSoundVolume", (*channel, f64::from(v)))?;
	}
	Ok(())
}
