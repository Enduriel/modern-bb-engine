//! The video mode in the settings file, as `video.*` keys. A key that's missing or
//! unreadable takes the value of the mode the game would start in without it; the
//! frame-rate cap follows VSync, as `setVideoMode` sets it.

use squirrel_bb::VideoMode;

use super::file::SettingsText;

/// The mode in `text`, with what it leaves out from `start` (the first start's mode).
pub fn read(text: &SettingsText, start: VideoMode) -> VideoMode {
	fn get<T: std::str::FromStr>(text: &SettingsText, key: &str, default: T) -> T {
		text.get(key).and_then(|v| v.parse().ok()).unwrap_or(default)
	}
	let mut mode = VideoMode {
		width: get(text, "video.width", start.width),
		height: get(text, "video.height", start.height),
		color_bits: get(text, "video.color_bits", start.color_bits),
		gamma: get(text, "video.gamma", start.gamma),
		brightness: get(text, "video.brightness", start.brightness),
		refresh_rate: get(text, "video.refresh_rate", start.refresh_rate),
		ui_scale: get(text, "video.ui_scale", start.ui_scale),
		scene_scale: get(text, "video.scene_scale", start.scene_scale),
		fullscreen: get(text, "video.fullscreen", start.fullscreen),
		borderless: get(text, "video.borderless", start.borderless),
		vsync: get(text, "video.vsync", start.vsync),
		depth_of_field: get(text, "video.depth_of_field", start.depth_of_field),
		..start
	};
	if mode.width <= 0 || mode.height <= 0 {
		(mode.width, mode.height) = (start.width, start.height);
	}
	mode.cap_frame_rate();
	mode
}

/// Write `mode` into `text`.
pub fn write(text: &mut SettingsText, mode: &VideoMode) {
	let entries: [(&str, String); 12] = [
		("video.width", mode.width.to_string()),
		("video.height", mode.height.to_string()),
		("video.color_bits", mode.color_bits.to_string()),
		("video.refresh_rate", mode.refresh_rate.to_string()),
		("video.fullscreen", mode.fullscreen.to_string()),
		("video.borderless", mode.borderless.to_string()),
		("video.vsync", mode.vsync.to_string()),
		("video.ui_scale", mode.ui_scale.to_string()),
		("video.scene_scale", mode.scene_scale.to_string()),
		("video.depth_of_field", mode.depth_of_field.to_string()),
		("video.gamma", mode.gamma.to_string()),
		("video.brightness", mode.brightness.to_string()),
	];
	for (key, value) in entries {
		text.set(key, &value);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_mode_round_trips() {
		let mode = VideoMode {
			width: 1600,
			height: 900,
			ui_scale: 1.25,
			fullscreen: false,
			borderless: false,
			vsync: false,
			depth_of_field: true,
			..VideoMode::first_start(2560, 1440, 32)
		};
		let mut text = SettingsText::default();
		write(&mut text, &mode);
		let back = read(&SettingsText::parse(&text.render()), VideoMode::first_start(1920, 1080, 32));
		assert_eq!(back, VideoMode { limit_frame_rate: true, frame_cap: 144, ..mode }, "VSync off caps it");
	}

	#[test]
	fn what_is_missing_or_unreadable_takes_the_first_starts() {
		let start = VideoMode::first_start(2560, 1440, 32);
		let text = SettingsText::parse("video.width = wide\nvideo.ui_scale = 1.5\nvideo.height = -3\n");
		let mode = read(&text, start);
		assert_eq!((mode.width, mode.height), (2560, 1440));
		assert_eq!(mode.ui_scale, 1.5);
		assert!(mode.fullscreen && mode.vsync && !mode.limit_frame_rate);
		assert_eq!(read(&SettingsText::default(), start), start, "no file: the first start");
	}
}
