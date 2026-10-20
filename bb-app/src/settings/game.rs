//! The rest of what the original keeps in its config file, in the settings file: the
//! gameplay options as `gameplay.<Name>` (the scripts' field names), the controls as
//! `controls.*`, the sound as `audio.*`. A key that's missing or unreadable takes the
//! value a fresh install has ([`GameSettings::FRESH_INSTALL`]).

use squirrel_bb::{GAMEPLAY_SETTINGS, GameSettings, SOUND_VOLUMES};

use super::file::SettingsText;

const HARDWARE_CURSOR: &str = "controls.hardware_cursor";
const DRAG_SCROLLING: &str = "controls.drag_scrolling";
const HARDWARE_SOUND: &str = "audio.hardware_sound";

fn gameplay_key(name: &str) -> String {
	format!("gameplay.{name}")
}

fn volume_key(name: &str) -> String {
	format!("audio.{name}_volume")
}

/// The settings in `text`, with what it leaves out from `base`.
pub fn read(text: &SettingsText, base: GameSettings) -> GameSettings {
	fn get<T: std::str::FromStr>(text: &SettingsText, key: &str, default: T) -> T {
		text.get(key).and_then(|v| v.parse().ok()).unwrap_or(default)
	}
	let mut s = base;
	for (i, name) in GAMEPLAY_SETTINGS.iter().enumerate() {
		s.gameplay[i] = get(text, &gameplay_key(name), s.gameplay[i]);
	}
	s.hardware_cursor = get(text, HARDWARE_CURSOR, s.hardware_cursor);
	s.drag_scrolling = get(text, DRAG_SCROLLING, s.drag_scrolling);
	s.hardware_sound = get(text, HARDWARE_SOUND, s.hardware_sound);
	for (i, (name, _)) in SOUND_VOLUMES.iter().enumerate() {
		let v: f32 = get(text, &volume_key(name), s.volumes[i]);
		if v.is_finite() {
			s.volumes[i] = v;
		}
	}
	s
}

/// Write `s` into `text`.
pub fn write(text: &mut SettingsText, s: &GameSettings) {
	for (name, v) in GAMEPLAY_SETTINGS.iter().zip(s.gameplay) {
		text.set(&gameplay_key(name), &v.to_string());
	}
	text.set(HARDWARE_CURSOR, &s.hardware_cursor.to_string());
	text.set(DRAG_SCROLLING, &s.drag_scrolling.to_string());
	text.set(HARDWARE_SOUND, &s.hardware_sound.to_string());
	for ((name, _), v) in SOUND_VOLUMES.iter().zip(s.volumes) {
		text.set(&volume_key(name), &v.to_string());
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_settings_round_trip() {
		let mut s = GameSettings::FRESH_INSTALL;
		s.set_gameplay("AutoLoot", true);
		s.set_gameplay("AdjustCameraLevel", false);
		s.drag_scrolling = false;
		s.hardware_cursor = true;
		s.hardware_sound = false;
		s.volumes = [0.33, 0.31, 0.5, 0.125];
		let mut text = SettingsText::default();
		write(&mut text, &s);
		let rendered = text.render();
		assert!(rendered.contains("gameplay.AutoLoot = true"), "{rendered}");
		assert!(rendered.contains("audio.master_volume = 0.33"), "{rendered}");
		assert_eq!(read(&SettingsText::parse(&rendered), GameSettings::FRESH_INSTALL), s);
	}

	#[test]
	fn what_is_missing_or_unreadable_is_a_fresh_installs() {
		let text = SettingsText::parse("gameplay.AutoLoot = maybe\naudio.music_volume = nan\ncontrols.drag_scrolling = false\n");
		let s = read(&text, GameSettings::FRESH_INSTALL);
		assert_eq!(s, GameSettings { drag_scrolling: false, ..GameSettings::FRESH_INSTALL });
	}
}
