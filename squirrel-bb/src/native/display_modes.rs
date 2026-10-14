//! The display modes behind `Settings.queryResolutions` and its index lookups.
//!
//! The original lists the monitor's modes, widest first (taller first among equal
//! widths), for each color depth (32-, then 16-, then 8-bit), and filters them by a
//! minimum width, height and depth (probed 2026-10-13). The host reports the monitor's
//! modes ([`super::VideoSettings::set_monitor_modes`]); headless, a fixed set of common
//! 32-bit modes stands in. The Options screen asks only for 32-bit modes, so the
//! missing 16/8-bit copies are invisible to it.
//!
//! The game's current size is listed too when the monitor has no such mode (a window
//! the player sized, which the original can't have), so the Options screen shows it.
//! A monitor that lists only its own mode (as on Wayland, where a game can't switch
//! modes) gets the common sizes that fit in it too: windows of those sizes, or
//! fullscreen scaled to the monitor.

/// One display mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DisplayMode {
	pub width: i64,
	pub height: i64,
	pub color_bits: i64,
}

/// The stand-in monitor's modes, in the original's order.
const MODES: &[(i64, i64)] = &[
	(1920, 1080),
	(1680, 1050),
	(1600, 1200),
	(1600, 900),
	(1440, 900),
	(1400, 1050),
	(1366, 768),
	(1280, 1024),
	(1280, 960),
	(1280, 800),
	(1280, 720),
	(1152, 864),
	(1024, 768),
	(800, 600),
	(640, 480),
];

/// What the list is made of: the monitor's resolutions (`None`: the stand-in list) and
/// the game's current size.
#[derive(Clone, Copy)]
pub(crate) struct Source<'a> {
	pub monitor: Option<&'a [(i64, i64)]>,
	pub current: (i64, i64),
}

/// The modes at least `min_width` × `min_height` with at least `min_bits` color bits,
/// as `Settings.queryResolutions(min_width, min_height, min_bits)` lists them.
pub(crate) fn query(source: Source, min_width: i64, min_height: i64, min_bits: i64) -> Vec<DisplayMode> {
	let mut sizes: Vec<(i64, i64)> = match source.monitor {
		None => MODES.to_vec(),
		Some(&[only]) => MODES.iter().copied().filter(|&(w, h)| w <= only.0 && h <= only.1).chain([only]).collect(),
		Some(modes) => modes.to_vec(),
	};
	if !sizes.contains(&source.current) {
		sizes.push(source.current);
	}
	// Widest first, taller first among equal widths.
	sizes.sort_by(|a, b| b.cmp(a));
	sizes.dedup();
	sizes
		.into_iter()
		.map(|(width, height)| DisplayMode {
			width,
			height,
			color_bits: 32,
		})
		.filter(|m| m.width >= min_width && m.height >= min_height && m.color_bits >= min_bits)
		.collect()
}

/// The index of `mode`'s resolution in [`query`]'s list, or `-1`.
pub(crate) fn index_of(source: Source, mode: DisplayMode, min_width: i64, min_height: i64, min_bits: i64) -> i64 {
	query(source, min_width, min_height, min_bits)
		.iter()
		.position(|m| *m == mode)
		.map_or(-1, |i| i as i64)
}

#[cfg(test)]
mod tests {
	use super::{DisplayMode, Source, index_of, query};

	const STAND_IN: Source = Source {
		monitor: None,
		current: (1920, 1080),
	};

	#[test]
	fn filters_by_the_minimum_and_keeps_the_order() {
		let modes = query(STAND_IN, 1024, 768, 32);
		assert_eq!(modes.first().map(|m| (m.width, m.height)), Some((1920, 1080)));
		assert!(modes.iter().all(|m| m.width >= 1024 && m.height >= 768));
		assert!(modes.windows(2).all(|w| (w[0].width, w[0].height) > (w[1].width, w[1].height)));
		assert!(query(STAND_IN, 4000, 3000, 32).is_empty());
	}

	#[test]
	fn the_monitors_modes_and_the_current_size() {
		let monitor = [(2560, 1440), (1920, 1080), (1280, 720)];
		let source = Source {
			monitor: Some(&monitor),
			current: (1700, 900),
		};
		let sizes: Vec<(i64, i64)> = query(source, 1024, 768, 32).iter().map(|m| (m.width, m.height)).collect();
		assert_eq!(sizes, [(2560, 1440), (1920, 1080), (1700, 900)], "the player's size in order; 1280 too small");
		let current = DisplayMode {
			width: 1700,
			height: 900,
			color_bits: 32,
		};
		assert_eq!(index_of(source, current, 1024, 768, 32), 2);
	}

	#[test]
	fn a_monitor_of_one_mode_gets_the_common_sizes_in_it() {
		let monitor = [(1700, 1000)];
		let source = Source {
			monitor: Some(&monitor),
			current: (1700, 1000),
		};
		let sizes: Vec<(i64, i64)> = query(source, 1280, 720, 32).iter().map(|m| (m.width, m.height)).collect();
		assert_eq!(sizes, [(1700, 1000), (1600, 900), (1440, 900), (1366, 768), (1280, 960), (1280, 800), (1280, 720)]);
	}

	#[test]
	fn finds_a_mode_by_resolution() {
		let full_hd = DisplayMode {
			width: 1920,
			height: 1080,
			color_bits: 32,
		};
		assert_eq!(index_of(STAND_IN, full_hd, 1024, 768, 32), 0);
		let odd = DisplayMode {
			width: 1234,
			height: 567,
			color_bits: 32,
		};
		assert_eq!(index_of(STAND_IN, odd, 1024, 768, 32), -1);
	}
}
