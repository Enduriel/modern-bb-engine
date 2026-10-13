//! UI parity against the original: drive our page through the screens
//! the goldens were captured on ([`bb_ui::host::goldens`]), snapshot each with the same
//! script, and list how it differs from the original's.
//!
//! Run: `cargo run -p bb-ui --features cef --example ui_parity -- [data-dir]
//!       [--goldens DIR] [--dump DIR] [--limit N]`
//!
//! `--dump DIR` also writes our snapshots there (`<screen>.json`) for a closer look.
//! The goldens come from `bb-ui/probes/ui_goldens.js` (see `bb-ui/probes/README.md`).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

use bb_engine::harness::STEAM_DATA_DIR;
use bb_ui::host::{HostConfig, UiSession, goldens};
use bb_ui::parity::{Difference, compare};

struct Options {
	data_dir: PathBuf,
	goldens: PathBuf,
	dump: Option<PathBuf>,
	limit: usize,
}

fn parse_options() -> Result<Options, String> {
	let mut options = Options {
		data_dir: PathBuf::from(STEAM_DATA_DIR),
		goldens: goldens::default_dir(),
		dump: None,
		limit: 40,
	};
	let mut args = std::env::args().skip(1);
	while let Some(arg) = args.next() {
		let mut value = || args.next().ok_or(format!("{arg} needs a value"));
		match arg.as_str() {
			"--goldens" => options.goldens = PathBuf::from(value()?),
			"--dump" => options.dump = Some(PathBuf::from(value()?)),
			"--limit" => options.limit = value()?.parse().map_err(|e| format!("--limit: {e}"))?,
			_ if !arg.starts_with("--") => options.data_dir = PathBuf::from(arg),
			_ => return Err(format!("unknown option {arg}")),
		}
	}
	Ok(options)
}

/// The kind of a difference, for the summary.
fn kind(difference: &Difference) -> String {
	match difference {
		Difference::Missing { .. } => "missing".into(),
		Difference::Extra { .. } => "extra".into(),
		Difference::Text { .. } => "text".into(),
		Difference::Rect { .. } => "box".into(),
		Difference::Style { property, .. } => format!("style {property}"),
	}
}

/// A difference with its path cut to the last three elements (`… > a > b > c`).
fn shorten(line: &str) -> String {
	let Some((kind, rest)) = line.split_once(": ") else {
		return line.to_string();
	};
	let (path, detail) = rest.split_once(": ").unwrap_or((rest, ""));
	let segments: Vec<&str> = path.split(" > ").collect();
	let tail = segments[segments.len().saturating_sub(3)..].join(" > ");
	let path = if segments.len() > 3 {
		format!("… > {tail}")
	} else {
		tail
	};
	if detail.is_empty() {
		format!("{kind}: {path}")
	} else {
		format!("{kind}: {path}: {detail}")
	}
}

fn run(options: Options) -> Result<(), String> {
	let golden_main = goldens::load(&options.goldens, goldens::SCREENS[0].0)?;
	let (width, height) = golden_main.viewport;
	let config = HostConfig {
		width: width as u32,
		height: height as u32,
		..HostConfig::default()
	};
	let session = UiSession::start(&options.data_dir, config, false)?;
	session.boot_menu()?;

	for (name, ours) in goldens::capture(&session)? {
		let golden = goldens::load(&options.goldens, name)?;
		if let Some(dir) = &options.dump {
			std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
			std::fs::write(dir.join(format!("{name}.json")), ours.to_json().to_text())
				.map_err(|e| e.to_string())?;
		}
		let differences = compare(&golden, &ours, &goldens::tolerance(name));
		let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
		for difference in &differences {
			*kinds.entry(kind(difference)).or_default() += 1;
		}
		println!("\n=== {name}: {} differences ===", differences.len());
		for (kind, count) in &kinds {
			println!("  {count:5}  {kind}");
		}
		for difference in differences.iter().take(options.limit) {
			println!("  {}", shorten(&difference.to_string()));
		}
	}
	Ok(())
}

fn main() -> ExitCode {
	if let Some(code) = bb_ui::host::run_subprocess() {
		return ExitCode::from(code.clamp(0, 255) as u8);
	}
	match parse_options().and_then(run) {
		Ok(()) => ExitCode::SUCCESS,
		Err(error) => {
			eprintln!("error: {error}");
			ExitCode::FAILURE
		}
	}
}
