//! The display modes behind `Settings.queryResolutions` and its index lookups.
//!
//! The original lists the monitor's modes, widest first (taller first among equal
//! widths), for each color depth (32-, then 16-, then 8-bit), and filters them by a
//! minimum width, height and depth (probed 2026-10-13). Headless there is no monitor,
//! so a fixed set of common 32-bit modes up to the default video mode stands in until
//! the windowing layer can enumerate the real one. The Options screen asks
//! only for 32-bit modes, so the missing 16/8-bit copies are invisible to it.

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

/// The modes at least `min_width` × `min_height` with at least `min_bits` color bits,
/// as `Settings.queryResolutions(min_width, min_height, min_bits)` lists them.
pub(crate) fn query(min_width: i64, min_height: i64, min_bits: i64) -> Vec<DisplayMode> {
	MODES
		.iter()
		.map(|&(width, height)| DisplayMode {
			width,
			height,
			color_bits: 32,
		})
		.filter(|m| m.width >= min_width && m.height >= min_height && m.color_bits >= min_bits)
		.collect()
}

/// The index of `mode`'s resolution in [`query`]'s list, or `-1`.
pub(crate) fn index_of(mode: DisplayMode, min_width: i64, min_height: i64, min_bits: i64) -> i64 {
	query(min_width, min_height, min_bits)
		.iter()
		.position(|m| *m == mode)
		.map_or(-1, |i| i as i64)
}

#[cfg(test)]
mod tests {
	use super::{DisplayMode, index_of, query};

	#[test]
	fn filters_by_the_minimum_and_keeps_the_order() {
		let modes = query(1024, 768, 32);
		assert_eq!(modes.first().map(|m| (m.width, m.height)), Some((1920, 1080)));
		assert!(modes.iter().all(|m| m.width >= 1024 && m.height >= 768));
		assert!(modes.windows(2).all(|w| (w[0].width, w[0].height) > (w[1].width, w[1].height)));
		assert!(query(4000, 3000, 32).is_empty());
	}

	#[test]
	fn finds_a_mode_by_resolution() {
		let full_hd = DisplayMode {
			width: 1920,
			height: 1080,
			color_bits: 32,
		};
		assert_eq!(index_of(full_hd, 1024, 768, 32), 0);
		let odd = DisplayMode {
			width: 1234,
			height: 567,
			color_bits: 32,
		};
		assert_eq!(index_of(odd, 1024, 768, 32), -1);
	}
}
