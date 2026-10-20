//! The size axis and what each glyph advance adds along it.
//!
//! The axis is `opsz` (optical size), 0 to [`AXIS_MAX`] px, which Blink sets to the
//! text's CSS font size (`font-optical-sizing: auto`, the default). Its normalized
//! coordinates are F2Dot14: [`STEPS`] steps a pixel. The regions, in order:
//!
//! - **Knots** (word deltas): for each whole size S, two hat functions peaking at
//!   S - 0.5 and just below S + 0.5, each spanning to its neighbours. They hold the
//!   advance that keeps the glyph at its whole-pixel width at size S over all of
//!   S's range [S - 0.5, S + 0.5): old WebKit rounds font sizes to whole pixels, so
//!   12.6px text advances as 13px text does. Between knots the advance is linear, an
//!   approximation of the 1/size curve that keeps the width (well under 0.01px a
//!   glyph off at fractional sizes).
//! - **At S itself**: delta units are whole numbers, and a whole pixel
//!   is rarely a whole number of units (2048/13). One narrow tent peaking at S adds
//!   whole units (word deltas; near 1px the 1/size curve bends too much for the
//!   knots); [`FRACTIONS`] narrow tents peaking just past S add fractions of a
//!   unit (byte deltas; each one's scalar at S is 8/(8+p)). Together they bring the advance to
//!   the whole pixel within about 1e-5px, always at or under it, so a line of whole
//!   pixels never comes out a 1/64px over (Blink lays text out in 1/64px).
//!
//! The tents at S reach [`NARROW`] steps (1/16px) to either side: a font size within
//! that of a whole one, but not whole, gets their contributions partly.

use std::sync::OnceLock;

/// The axis's maximum, in pixels: sizes from it up get the font's own advances.
pub const AXIS_MAX: u16 = 128;
/// F2Dot14 steps a pixel.
pub const STEPS: i32 = 16384 / AXIS_MAX as i32;
/// The whole sizes that get whole-pixel advances.
pub const SIZES: std::ops::RangeInclusive<i32> = 1..=AXIS_MAX as i32 - 1;
/// How far the tents at a whole size reach, in steps.
const NARROW: i32 = 8;
/// Where the fraction tents peak, in steps past their size.
const FRACTIONS: [i32; 4] = [3, 5, 6, 7];
/// The largest fraction-tent delta.
const MAX_FRACTION_DELTA: i8 = 9;

/// A region: start, peak and end on the axis, in normalized (F2Dot14) steps.
pub type Region = (i32, i32, i32);

/// The number of word-delta regions (the knots and the whole tents).
pub fn word_count() -> usize {
	SIZES.count() * 3
}

/// The knots' peaks, in steps, in order.
fn knots() -> impl Iterator<Item = i32> {
	SIZES.flat_map(|size| [size * STEPS - STEPS / 2, size * STEPS + STEPS / 2 - 1])
}

/// The regions: the knots, the sizes' whole tents, then each size's fraction tents.
pub fn regions() -> Vec<Region> {
	let peaks: Vec<i32> = knots().collect();
	let mut regions = Vec::with_capacity(peaks.len() + SIZES.count() * (1 + FRACTIONS.len()));
	for (i, &peak) in peaks.iter().enumerate() {
		let start = if i == 0 { 0 } else { peaks[i - 1] };
		let end = peaks.get(i + 1).copied().unwrap_or(16384);
		regions.push((start, peak, end));
	}
	for size in SIZES {
		let at = size * STEPS;
		regions.push((at - NARROW, at, at + NARROW));
	}
	for size in SIZES {
		let at = size * STEPS;
		for p in FRACTIONS {
			regions.push((at - NARROW, at + p, at + NARROW));
		}
	}
	regions
}

/// The fraction tents' sums at their size, `(value, deltas)`, sorted by value.
fn fraction_sums() -> &'static [(f64, [i8; FRACTIONS.len()])] {
	static SUMS: OnceLock<Vec<(f64, [i8; FRACTIONS.len()])>> = OnceLock::new();
	SUMS.get_or_init(|| {
		let range = -MAX_FRACTION_DELTA..=MAX_FRACTION_DELTA;
		let width = range.clone().count();
		let mut sums = Vec::with_capacity(width.pow(FRACTIONS.len() as u32));
		for index in 0..width.pow(FRACTIONS.len() as u32) {
			let mut deltas = [0i8; FRACTIONS.len()];
			let mut rest = index;
			for delta in &mut deltas {
				*delta = (rest % width) as i8 - MAX_FRACTION_DELTA;
				rest /= width;
			}
			sums.push((fraction_sum(&deltas), deltas));
		}
		sums.sort_by(|a, b| a.0.total_cmp(&b.0));
		sums
	})
}

/// What fraction tents with `deltas` add at their size.
fn fraction_sum(deltas: &[i8]) -> f64 {
	deltas
		.iter()
		.zip(FRACTIONS)
		.map(|(&delta, p)| delta as f64 * NARROW as f64 / (NARROW + p) as f64)
		.sum()
}

/// The deltas of a glyph advancing `advance` units in a font of `units_per_em`, one
/// per region of [`regions`]: the first [`word_count`] as `i16`, the rest as `i8`.
pub fn deltas(advance: u16, units_per_em: u16) -> (Vec<i16>, Vec<i8>) {
	let advance = advance as f64;
	let em = units_per_em as f64;
	// Whole pixels at size S.
	let pixels = |size: i32| (advance * size as f64 / em).round();
	let knot_sizes = SIZES.flat_map(|size| [size, size]);
	let mut words: Vec<i16> = knots()
		.zip(knot_sizes)
		.map(|(peak, size)| {
			let at = peak as f64 / STEPS as f64;
			let units = pixels(size) * em / at - advance;
			units.round().clamp(i16::MIN as f64, i16::MAX as f64) as i16
		})
		.collect();
	let sums = fraction_sums();
	let mut bytes = Vec::with_capacity(SIZES.count() * FRACTIONS.len());
	for (i, size) in SIZES.enumerate() {
		// The knots' hats at S: 63/127 of the one below, 64/127 of the one above.
		let (below, above) = (words[2 * i] as f64, words[2 * i + 1] as f64);
		let half = (STEPS / 2) as f64;
		let between = (below * (half - 1.0) + above * half) / (2.0 * half - 1.0);
		let wanted = pixels(size) * em / size as f64 - advance - between;
		let whole = wanted.floor();
		let fraction = wanted - whole;
		// The largest sum at or under the fraction (the smallest is negative).
		let at = sums.partition_point(|(value, _)| *value <= fraction);
		let (_, fractions) = sums[at.max(1) - 1];
		words.push(whole.clamp(i16::MIN as f64, i16::MAX as f64) as i16);
		bytes.extend_from_slice(&fractions);
	}
	(words, bytes)
}

#[cfg(test)]
mod tests {
	use super::{AXIS_MAX, MAX_FRACTION_DELTA, Region, SIZES, STEPS, deltas, fraction_sums, regions, word_count};

	/// A region's scalar at `at` (OpenType's tent rule, one axis).
	fn scalar((start, peak, end): Region, at: f64) -> f64 {
		let (start, peak, end) = (start as f64, peak as f64, end as f64);
		if at == peak {
			1.0
		} else if at <= start || at >= end {
			0.0
		} else if at < peak {
			(at - start) / (peak - start)
		} else {
			(end - at) / (end - peak)
		}
	}

	/// The advance, in pixels, of a glyph of `advance` units at `size` px.
	fn pixels(advance: u16, em: u16, size: f64) -> f64 {
		let (words, bytes) = deltas(advance, em);
		let at = size * STEPS as f64;
		let units: f64 = regions()
			.into_iter()
			.zip(words.iter().map(|&d| d as f64).chain(bytes.iter().map(|&d| d as f64)))
			.map(|(region, delta)| scalar(region, at) * delta)
			.sum();
		(advance as f64 + units) * size / em as f64
	}

	#[test]
	fn regions_are_well_formed() {
		let regions = regions();
		assert_eq!(regions.len(), word_count() + SIZES.count() * 4);
		for &(start, peak, end) in &regions {
			assert!(0 <= start && start < peak && peak < end && end <= 16384, "{start} {peak} {end}");
		}
		assert_eq!(STEPS * AXIS_MAX as i32, 16384);
	}

	#[test]
	fn fraction_sums_are_fine_enough() {
		let sums = fraction_sums();
		let finest = sums.windows(2).filter(|w| w[0].0 >= 0.0 && w[1].0 <= 1.0);
		let widest = finest.map(|w| w[1].0 - w[0].0).fold(0.0, f64::max);
		assert!(widest < 0.003, "{widest}");
		assert!(sums.iter().all(|(_, d)| d.iter().all(|d| d.abs() <= MAX_FRACTION_DELTA)));
	}

	#[test]
	fn whole_sizes_advance_whole_pixels_or_a_hair_under() {
		for em in [1000, 1024, 2048] {
			for advance in [0, 1, 37, 229, 500, 569, 600, 1229, 2048, 4000] {
				for size in SIZES {
					let wanted = (advance as f64 * size as f64 / em as f64).round();
					let got = pixels(advance, em, size as f64);
					let off = got - wanted;
					assert!((-2e-4..=1e-9).contains(&off), "{advance}/{em} at {size}px: {got} for {wanted}");
				}
			}
		}
	}

	#[test]
	fn fractional_sizes_advance_as_the_rounded_size() {
		for size in [12.5, 12.75, 13.25, 13.49, 28.4, 39.6] {
			let rounded = (size + 0.5_f64).floor();
			for advance in [229, 569, 1229] {
				let wanted = (advance as f64 * rounded / 2048.0).round();
				let got = pixels(advance, 2048, size);
				assert!((got - wanted).abs() < 0.01, "{advance} at {size}px: {got} for {wanted}");
			}
		}
	}

	#[test]
	fn sizes_from_the_axis_maximum_keep_the_fonts_advances() {
		assert_eq!(pixels(569, 2048, 128.0), 569.0 * 128.0 / 2048.0);
	}
}
