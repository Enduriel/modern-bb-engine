//! How the world textures' candidate formats (`bb_render::world_textures`) compare on
//! an image: each one's bytes with mips, encode time, and error against the source on
//! every mip level (PSNR over RGB, mean and largest channel difference), from the
//! reference decoders. Optionally writes each candidate's level 0 decoded.
//!
//! ```text
//! cargo run --release -p bb-render --example world_texture_formats -- IMAGE.png [--out DIR]
//! ```

use std::process::ExitCode;
use std::time::Instant;

use bb_assets::Image;
use bb_render::block::{self, Plane};

/// A candidate's levels, decoded back to RGBA, with its bytes and encode time.
struct Encoded {
	name: &'static str,
	levels: Vec<Image>,
	bytes: u64,
	millis: f64,
}

fn grey(image: &Image) -> Image {
	Image {
		width: image.width,
		height: image.height,
		rgba: image.rgba.chunks_exact(4).flat_map(|t| [t[0], t[0], t[0], 255]).collect(),
	}
}

/// `image` stretched to twice its size, bilinear (texel centres), as a sampler draws a
/// half-size texture at the full one's scale, wrapping at the edges.
fn doubled(image: &Image) -> Image {
	let (w, h) = (image.width * 2, image.height * 2);
	let texel = |x: i64, y: i64, c: usize| {
		let (x, y) = (x.rem_euclid(image.width as i64) as u32, y.rem_euclid(image.height as i64) as u32);
		f32::from(image.rgba[((y * image.width + x) * 4) as usize + c])
	};
	let mut rgba = Vec::with_capacity((w * h * 4) as usize);
	for y in 0..h {
		for x in 0..w {
			let (fx, fy) = ((x as f32 + 0.5) / 2.0 - 0.5, (y as f32 + 0.5) / 2.0 - 0.5);
			let (x0, y0) = (fx.floor() as i64, fy.floor() as i64);
			let (tx, ty) = (fx - fx.floor(), fy - fy.floor());
			for c in 0..4 {
				let top = texel(x0, y0, c) * (1.0 - tx) + texel(x0 + 1, y0, c) * tx;
				let bottom = texel(x0, y0 + 1, c) * (1.0 - tx) + texel(x0 + 1, y0 + 1, c) * tx;
				rgba.push((top * (1.0 - ty) + bottom * ty).round() as u8);
			}
		}
	}
	Image { width: w, height: h, rgba }
}

fn encode(name: &'static str, source: &Image, threads: usize) -> Encoded {
	let start = Instant::now();
	let half = name.ends_with("-half");
	let top = if half { bb_assets::mips::halve(source) } else { source.clone() };
	let chain = bb_assets::mips::mip_chain(&top);
	let mut bytes = 0u64;
	let mut levels = Vec::new();
	for level in &chain {
		let plane = Plane::rgba(level);
		let (across, down) = block::blocks(level.width, level.height);
		let blocks = u64::from(across * down) * block::BLOCK_BYTES as u64;
		let texels = u64::from(level.width * level.height);
		levels.push(match name.trim_end_matches("-half") {
			"rgba8" => {
				bytes += texels * 4;
				level.clone()
			}
			"r8" => {
				bytes += texels;
				grey(level)
			}
			"bc1" => {
				bytes += blocks;
				block::decode_bc1(level.width, level.height, &block::encode_bc1(&plane, threads))
			}
			"bc4" => {
				bytes += blocks;
				block::decode_bc4(level.width, level.height, &block::encode_bc4(&plane, threads))
			}
			other => panic!("no candidate {other}"),
		});
	}
	let millis = start.elapsed().as_secs_f64() * 1000.0;
	if half {
		// Compared at the source's levels: each stored level drawn twice as large.
		let first = doubled(&levels[0]);
		levels.insert(0, first);
	}
	Encoded { name, levels, bytes, millis }
}

/// PSNR over RGB, mean and largest channel difference.
fn error(got: &Image, want: &Image) -> (f64, f64, u8) {
	let (mut squares, mut sum, mut max, mut n) = (0.0f64, 0.0f64, 0u8, 0usize);
	for (a, b) in got.rgba.chunks_exact(4).zip(want.rgba.chunks_exact(4)) {
		for c in 0..3 {
			let d = a[c].abs_diff(b[c]);
			squares += f64::from(d).powi(2);
			sum += f64::from(d);
			max = max.max(d);
			n += 1;
		}
	}
	let mse = squares / n as f64;
	let psnr = if mse == 0.0 { f64::INFINITY } else { 10.0 * (255.0f64.powi(2) / mse).log10() };
	(psnr, sum / n as f64, max)
}

fn run() -> Result<(), String> {
	let args: Vec<String> = std::env::args().skip(1).collect();
	let path = args.first().ok_or("usage: world_texture_formats IMAGE.png [--out DIR]")?;
	let out = args.iter().position(|a| a == "--out").and_then(|i| args.get(i + 1));
	let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
	let source = bb_assets::decode_png(&bytes)?;
	let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(8);
	let chain = bb_assets::mips::mip_chain(&source);
	println!("{path}: {}×{}, {} levels, {threads} threads", source.width, source.height, chain.len());
	for name in ["rgba8", "r8", "bc1", "bc4", "rgba8-half", "r8-half", "bc1-half", "bc4-half"] {
		let encoded = encode(name, &source, threads);
		let mib = encoded.bytes as f64 / (1024.0 * 1024.0);
		print!("{:<11} {mib:7.2} MiB {:7.1} ms |", encoded.name, encoded.millis);
		for (level, want) in chain.iter().enumerate().take(6) {
			let (psnr, mean, max) = error(&encoded.levels[level], want);
			print!(" L{level} {psnr:5.1} dB {mean:4.2} {max:3} |");
		}
		println!();
		if let Some(dir) = out {
			std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
			let file = std::path::Path::new(dir).join(format!("{name}.png"));
			bb_render::save_png(&file, &encoded.levels[0])?;
		}
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
