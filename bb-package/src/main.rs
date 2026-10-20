//! `bb-package`: a release package of bb for one target.
//!
//! ```text
//! cargo run --release -p bb-package -- [--target TRIPLE] [--out DIR] [--cef-dir DIR]
//!     [--cache DIR] [--no-build] [--no-archive]
//! ```
//!
//! 1. Builds `bb` (`bb-app` with `ui`) in release for the target, its rpath set to its
//!    own folder (`BB_PACKAGE_RPATH=origin`).
//! 2. Fetches the pinned `steamworks-sys` crate from crates.io (once, into the cache),
//!    checks its SHA-256 and that of the target's Steam library inside it against the
//!    pins in `bb_steam::pinned`, and refuses anything else ([`steam_library`]).
//! 3. Lays out `<out>/bb-<target>/`: `bb`, the CEF files it runs with (stripped on
//!    Linux, English locale only), Steam's library, and the licences ([`layout`]); on
//!    macOS `bb.app` with all of that inside but the licences ([`mac_app`]).
//! 4. Packs it as `<out>/bb-<target>.tar.gz` (`.zip` for Windows) and prints the sizes.
//!
//! Needs `curl` and `tar` on the path (both ship with Windows 10+, macOS and Linux
//! distributions), and `strip` on Linux. See `docs/packaging.md`.

mod layout;
mod mac_app;
mod steam_library;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

struct Options {
	target: String,
	out: PathBuf,
	cef_dir: Option<PathBuf>,
	cache: PathBuf,
	build: bool,
	archive: bool,
}

/// The workspace folder (`modern_bb_engine`).
fn workspace() -> PathBuf {
	Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("bb-package sits in the workspace").to_path_buf()
}

/// Cargo's target folder for this workspace.
fn target_dir() -> PathBuf {
	std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from).unwrap_or_else(|| workspace().join("target"))
}

fn options() -> Result<Options, String> {
	let mut options = Options {
		target: env!("BB_PACKAGE_HOST").to_string(),
		out: target_dir().join("package"),
		cef_dir: None,
		cache: target_dir().join("package-cache"),
		build: true,
		archive: true,
	};
	let mut args = std::env::args().skip(1);
	while let Some(arg) = args.next() {
		let mut value = || args.next().ok_or(format!("{arg} needs a value"));
		match arg.as_str() {
			"--target" => options.target = value()?,
			"--out" => options.out = PathBuf::from(value()?),
			"--cef-dir" => options.cef_dir = Some(PathBuf::from(value()?)),
			"--cache" => options.cache = PathBuf::from(value()?),
			"--no-build" => options.build = false,
			"--no-archive" => options.archive = false,
			_ => return Err(format!("unknown argument {arg}")),
		}
	}
	Ok(options)
}

/// Run `command`, failing with its name if it fails.
pub fn run(command: &mut Command) -> Result<(), String> {
	let name = format!("{:?}", command.get_program());
	let status = command.status().map_err(|e| format!("{name}: {e}"))?;
	if status.success() { Ok(()) } else { Err(format!("{name} failed ({status})")) }
}

/// Build bb for `target` and return its path.
fn build_bb(target: &str, enabled: bool) -> Result<PathBuf, String> {
	if enabled {
		let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
		run(Command::new(cargo)
			.current_dir(workspace())
			.args(["build", "--release", "-p", "bb-app", "--features", "ui", "--bin", "bb", "--target", target])
			.env("BB_PACKAGE_RPATH", "origin"))?;
	}
	let exe = target_dir().join(target).join("release").join(layout::exe_name(target));
	if exe.is_file() { Ok(exe) } else { Err(format!("no bb at {}", exe.display())) }
}

/// The size of everything under `path`, in bytes.
fn size_of(path: &Path) -> u64 {
	match std::fs::symlink_metadata(path) {
		Ok(meta) if meta.is_dir() => std::fs::read_dir(path).map_or(0, |entries| entries.flatten().map(|e| size_of(&e.path())).sum()),
		Ok(meta) => meta.len(),
		Err(_) => 0,
	}
}

fn mib(bytes: u64) -> String {
	format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
}

fn package() -> Result<(), String> {
	let options = options()?;
	let target = options.target.as_str();
	let platform = layout::Platform::of(target)?;
	let cef_dir = match &options.cef_dir {
		Some(dir) => dir.clone(),
		None => layout::find_cef_dir(&workspace().join("target/cef"), platform)?,
	};
	let bb = build_bb(target, options.build)?;
	let steam = steam_library::fetch(target, &options.cache)?;
	let name = format!("bb-{target}");
	let dir = options.out.join(&name);
	layout::lay_out(&dir, platform, &bb, &cef_dir, &steam)?;
	println!("package: {} ({})", dir.display(), mib(size_of(&dir)));
	if options.archive {
		let archive = layout::archive(&options.out, &name, platform)?;
		println!("archive: {} ({})", archive.display(), mib(size_of(&archive)));
	}
	Ok(())
}

fn main() -> ExitCode {
	match package() {
		Ok(()) => ExitCode::SUCCESS,
		Err(e) => {
			eprintln!("bb-package: {e}");
			ExitCode::FAILURE
		}
	}
}
