//! Where a frame of ours differs from a screenshot of the original (frame parity).
//!
//! ```text
//! cargo run --release -p bb-render --example frame_diff -- OURS.png ORIGINAL.png HEAT.png
//!     [--tolerance N] [--noise ORIGINAL2.png] [--noise-ours OURS2.png]
//! ```
//!
//! `--noise` is another run of the original, `--noise-ours` one of ours, each as often
//! as there are runs: what differs between an engine's own runs is left out (blue in
//! the heatmap), with 2 pixels around it. Use several runs of the original: its random
//! detail looks (a brush flip, say) agree between two runs often enough by chance to
//! show as differences.
//! Prints how many pixels differ by more than N (default 24) in a colour channel,
//! and the regions they gather in (16-pixel cells where more than 5% differ, joined),
//! most first; writes HEAT.png, ours dimmed to grey with the differing pixels in red,
//! and for the 12 largest regions HEAT-region-N.png: ours and the original side by side.

use std::process::ExitCode;

use bb_assets::Image;
use bb_render::frame_diff::{self, CELL, Region};

fn load(path: &str) -> Result<Image, String> {
	let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
	bb_assets::decode_png(&bytes).map_err(|e| format!("{path}: {e}"))
}

/// `region` with a margin, from `ours` and `original` side by side (a gap between).
fn side_by_side(ours: &Image, original: &Image, region: &Region) -> Image {
	const MARGIN: u32 = 24;
	const GAP: u32 = 8;
	let x0 = region.x.saturating_sub(MARGIN);
	let y0 = region.y.saturating_sub(MARGIN);
	let x1 = (region.x + region.width + MARGIN).min(ours.width);
	let y1 = (region.y + region.height + MARGIN).min(ours.height);
	let (w, h) = (x1 - x0, y1 - y0);
	let width = w * 2 + GAP;
	let mut rgba = vec![255u8; (width * h * 4) as usize];
	for (n, image) in [ours, original].into_iter().enumerate() {
		for y in 0..h {
			let from = (((y0 + y) * image.width + x0) * 4) as usize;
			let to = ((y * width + n as u32 * (w + GAP)) * 4) as usize;
			rgba[to..to + (w * 4) as usize]
				.copy_from_slice(&image.rgba[from..from + (w * 4) as usize]);
		}
	}
	Image {
		width,
		height: h,
		rgba,
	}
}

fn run() -> Result<(), String> {
	let args: Vec<String> = std::env::args().skip(1).collect();
	let [ours, original, heat, rest @ ..] = args.as_slice() else {
		return Err("usage: frame_diff OURS.png ORIGINAL.png HEAT.png [--tolerance N] [--noise ORIGINAL2.png] [--noise-ours OURS2.png]".into());
	};
	let (mut tolerance, mut noise_original, mut noise_ours) = (24u8, Vec::new(), Vec::new());
	let mut rest = rest.iter();
	while let Some(flag) = rest.next() {
		let value = rest.next().ok_or(format!("{flag} needs a value"))?;
		match flag.as_str() {
			"--tolerance" => {
				tolerance = value
					.parse()
					.map_err(|_| format!("not a tolerance: {value}"))?
			}
			"--noise" => noise_original.push(value),
			"--noise-ours" => noise_ours.push(value),
			_ => return Err(format!("unknown option {flag}")),
		}
	}
	let (ours_image, original_image) = (load(ours)?, load(original)?);
	let mut noise: Option<Vec<bool>> = None;
	let runs = noise_original
		.iter()
		.map(|r| (r, &original_image))
		.chain(noise_ours.iter().map(|r| (r, &ours_image)));
	for (again, same) in runs {
		let mask = frame_diff::noise_mask(same, &load(again)?, tolerance, 2)?;
		noise = Some(match noise {
			Some(n) => n.iter().zip(&mask).map(|(a, b)| *a || *b).collect(),
			None => mask,
		});
	}
	let d = frame_diff::diff(
		&ours_image,
		&original_image,
		noise.as_deref(),
		tolerance,
		0.05,
	)?;
	bb_render::save_png(heat, &d.heat)?;
	let stem = heat.strip_suffix(".png").unwrap_or(heat);
	for (n, region) in d.regions.iter().take(12).enumerate() {
		bb_render::save_png(
			format!("{stem}-region-{n:02}.png"),
			&side_by_side(&ours_image, &original_image, region),
		)?;
	}
	println!(
		"{} of {} pixels differ ({:.2}%) beyond {tolerance}, {} left out as noise; {} regions of {CELL}px cells:",
		d.differing,
		d.total,
		100.0 * d.differing as f64 / d.total as f64,
		d.masked,
		d.regions.len()
	);
	for r in &d.regions {
		println!(
			"  {:>7} px at {},{} {}x{}",
			r.pixels, r.x, r.y, r.width, r.height
		);
	}
	Ok(())
}

fn main() -> ExitCode {
	match run() {
		Ok(()) => ExitCode::SUCCESS,
		Err(e) => {
			eprintln!("error: {e}");
			ExitCode::FAILURE
		}
	}
}
