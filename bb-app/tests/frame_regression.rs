//! Rendering regressions: the frame parity scenes (`bb_app::frame_scenes`) against our own
//! from before, so a change to what the game draws shows here without the original
//! (`parity/frames/` compares with the original's, which takes probe runs).
//!
//! The goldens are PNGs outside the repository, in the folder `BB_FRAME_GOLDENS` names.
//! Skips, saying so, without them or without a Steam
//! install. To take them again after a change that is meant to show:
//!
//! ```text
//! BB_FRAME_GOLDENS_UPDATE=1 cargo test --release -p bb-app --features ui --test frame_regression
//! ```
//!
//! The engine's own randomness (clouds, map details) is seeded (`BB_ENGINE_SEED`), so two
//! runs draw the same. What still moves, and how much of each scene may differ, is in
//! `parity/frames/regression.txt`. A failing scene's heatmap (and crops) are written next
//! to the test's build (`frame_regression/`).
//!
//! A plain `main` (`harness = false`): Chromium re-runs this binary for its child
//! processes.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use bb_assets::Image;
use bb_engine::harness::STEAM_DATA_DIR;
use bb_render::{Gpu, frame_diff, save_png};

const NAME: &str = "frames_match_ours_from_before";
const SCREEN: (i64, i64) = (2560, 1440);
/// A pixel differs when a colour channel is off by more than this.
const CHANNEL_TOLERANCE: u8 = 24;

fn goldens_dir() -> PathBuf {
	// Unset: a path that doesn't exist, so the test says what it lacks.
	std::env::var_os("BB_FRAME_GOLDENS").map_or_else(|| PathBuf::from("<BB_FRAME_GOLDENS is unset>"), PathBuf::from)
}

/// A scene's allowance: the share of its pixels that may differ (%), and rectangles left
/// out (`x,y,w,h`).
struct Allowance {
	percent: f64,
	masks: Vec<(u32, u32, u32, u32)>,
}

/// `parity/frames/regression.txt`: a line per scene, `scene percent [x,y,w,h ...]`;
/// `default` for the scenes not listed.
fn allowances() -> Result<Vec<(String, Allowance)>, String> {
	let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../parity/frames/regression.txt");
	let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
	let mut out = Vec::new();
	for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
		let mut fields = line.split_whitespace();
		let scene = fields.next().unwrap_or_default().to_string();
		let percent = fields
			.next()
			.and_then(|p| p.parse().ok())
			.ok_or(format!("{}: no percent in {line:?}", path.display()))?;
		let mut masks = Vec::new();
		for mask in fields {
			let n: Vec<u32> = mask.split(',').filter_map(|v| v.parse().ok()).collect();
			let [x, y, w, h] = n[..] else {
				return Err(format!("{}: not x,y,w,h: {mask}", path.display()));
			};
			masks.push((x, y, w, h));
		}
		out.push((scene, Allowance { percent, masks }));
	}
	Ok(out)
}

fn allowance<'a>(all: &'a [(String, Allowance)], scene: &str) -> Option<&'a Allowance> {
	all.iter()
		.find(|(s, _)| s == scene)
		.or_else(|| all.iter().find(|(s, _)| s == "default"))
		.map(|(_, a)| a)
}

/// The pixels the masks leave out, as `frame_diff` takes them.
fn masked(image: &Image, masks: &[(u32, u32, u32, u32)]) -> Vec<bool> {
	let mut out = vec![false; (image.width * image.height) as usize];
	for &(x, y, w, h) in masks {
		for py in y..(y + h).min(image.height) {
			for px in x..(x + w).min(image.width) {
				out[(py * image.width + px) as usize] = true;
			}
		}
	}
	out
}

fn run(goldens: &Path, update: bool) -> Result<Vec<String>, String> {
	let gpu = Gpu::headless_with(bb_app::front::GPU_FEATURES)?;
	let all = allowances()?;
	let report = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("frame_regression");
	std::fs::create_dir_all(&report).map_err(|e| e.to_string())?;
	let mut failures = Vec::new();
	bb_app::frame_scenes::shoot(&gpu, SCREEN, |scene, ours| {
		let golden_path = goldens.join(format!("{scene}.png"));
		if update {
			save_png(&golden_path, &ours)?;
			println!("{scene}: {}", golden_path.display());
			return Ok(());
		}
		let Ok(bytes) = std::fs::read(&golden_path) else {
			failures.push(format!("{scene}: no golden at {}", golden_path.display()));
			return Ok(());
		};
		let golden = bb_assets::decode_png(&bytes).map_err(|e| format!("{}: {e}", golden_path.display()))?;
		let allowed = allowance(&all, scene).ok_or(format!("no allowance for {scene} and no default"))?;
		let mask = masked(&ours, &allowed.masks);
		let d = frame_diff::diff(&ours, &golden, Some(&mask), CHANNEL_TOLERANCE, 0.05)?;
		let percent = 100.0 * d.differing as f64 / d.total as f64;
		println!("{scene}: {percent:.3}% differ (allowed {}%)", allowed.percent);
		if percent > allowed.percent {
			let heat = report.join(format!("{scene}-heat.png"));
			save_png(&heat, &d.heat)?;
			save_png(report.join(format!("{scene}-ours.png")), &ours)?;
			let regions: Vec<String> = d
				.regions
				.iter()
				.take(5)
				.map(|r| format!("{} px at {},{} {}x{}", r.pixels, r.x, r.y, r.width, r.height))
				.collect();
			failures.push(format!(
				"{scene}: {percent:.3}% of its pixels differ, {}% allowed; largest regions: {}; heatmap {}",
				allowed.percent,
				regions.join(", "),
				heat.display()
			));
		}
		Ok(())
	})?;
	Ok(failures)
}

fn main() -> ExitCode {
	// The engine's own randomness, seeded before anything starts (the browser's
	// processes inherit it, harmlessly).
	if std::env::var_os("BB_ENGINE_SEED").is_none() {
		// SAFETY: first thing in `main`, so no other thread exists yet.
		unsafe { std::env::set_var("BB_ENGINE_SEED", "20261018") };
	}
	if let Some(code) = bb_ui::host::run_subprocess() {
		return ExitCode::from(code.clamp(0, 255) as u8);
	}
	// A runner's `--list` (cargo-nextest's), answered before a display starts.
	if let Some(code) = bb_engine::harness::list_case(NAME) {
		return code;
	}
	// Chromium paints the page through a display connection: a headless gamescope's.
	bb_ui::host::with_display(run_main)
}

fn run_main() -> ExitCode {
	if !Path::new(STEAM_DATA_DIR).exists() {
		return bb_engine::harness::skip_case(NAME, format!("{STEAM_DATA_DIR} not present"));
	}
	let goldens = goldens_dir();
	let update = std::env::var_os("BB_FRAME_GOLDENS_UPDATE").is_some();
	if update {
		if let Err(e) = std::fs::create_dir_all(&goldens) {
			println!("test {NAME} ... FAILED\n{}: {e}", goldens.display());
			return ExitCode::FAILURE;
		}
	} else if std::fs::read_dir(&goldens).map_or(true, |mut d| d.next().is_none()) {
		return bb_engine::harness::skip_case(
			NAME,
			format!("no frame goldens at {} (take them with BB_FRAME_GOLDENS_UPDATE=1)", goldens.display()),
		);
	}
	match run(&goldens, update) {
		Ok(failures) if failures.is_empty() => {
			println!("test {NAME} ... ok{}", if update { " (goldens taken)" } else { "" });
			ExitCode::SUCCESS
		}
		Ok(failures) => {
			println!("test {NAME} ... FAILED\n{}", failures.join("\n"));
			ExitCode::FAILURE
		}
		Err(e) => {
			println!("test {NAME} ... FAILED\n{e}");
			ExitCode::FAILURE
		}
	}
}
