//! What a package holds for each platform, and laying it out.
//!
//! Linux (verified): `bb` and `libcef.so` stripped, CEF's resources (`icudtl.dat`,
//! `resources.pak`, `chrome_100_percent.pak`, `chrome_200_percent.pak`,
//! `v8_context_snapshot.bin`), the English locale only (the page is the game's own, and
//! CEF falls back to `en-US` for any other system locale), Steam's library, and the
//! notices. Left out: `chrome-sandbox` (the host runs CEF without its sandbox) and CEF's
//! SwiftShader/Vulkan files (the host turns the GPU off; see `docs/packaging.md` for the
//! check that nothing loads them).
//!
//! Windows (not yet built or run): every DLL and resource at the top of CEF's folder,
//! the same locale. macOS needs an app bundle with CEF's framework and helper apps
//! and is refused for now.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::steam_library::SteamLibrary;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
	Linux,
	Windows,
}

impl Platform {
	pub fn of(target: &str) -> Result<Platform, String> {
		if target.contains("-linux-") {
			Ok(Platform::Linux)
		} else if target.contains("-windows-") {
			Ok(Platform::Windows)
		} else if target.contains("-apple-") {
			Err("macOS packages need an app bundle with CEF's framework and helpers, not built yet".into())
		} else {
			Err(format!("no package layout for {target}"))
		}
	}

	/// CEF's name for the platform in its folder names (`cef_linux_x86_64`).
	fn cef_name(self) -> &'static str {
		match self {
			Platform::Linux => "linux",
			Platform::Windows => "windows",
		}
	}
}

/// The executable's file name for `target`.
pub fn exe_name(target: &str) -> &'static str {
	if target.contains("-windows-") { "bb.exe" } else { "bb" }
}

/// The CEF distribution the build downloaded for `platform` (`<root>/<version>/cef_<os>_<arch>`);
/// exactly one, or `--cef-dir` must say which.
pub fn find_cef_dir(root: &Path, platform: Platform) -> Result<PathBuf, String> {
	let prefix = format!("cef_{}_", platform.cef_name());
	let mut found = Vec::new();
	for version in fs::read_dir(root).map_err(|e| format!("{}: {e} (build bb with --features ui first)", root.display()))?.flatten() {
		for dir in fs::read_dir(version.path()).into_iter().flatten().flatten() {
			if dir.file_name().to_string_lossy().starts_with(&prefix) && dir.path().is_dir() {
				found.push(dir.path());
			}
		}
	}
	match found.as_slice() {
		[one] => Ok(one.clone()),
		[] => Err(format!("no {prefix}* CEF distribution under {}", root.display())),
		_ => Err(format!("several CEF distributions under {}; pick one with --cef-dir: {found:?}", root.display())),
	}
}

/// CEF's files for `platform` (relative to its folder), and whether each is stripped.
fn cef_files(cef_dir: &Path, platform: Platform) -> Result<Vec<(PathBuf, bool)>, String> {
	match platform {
		Platform::Linux => Ok(["libcef.so", "icudtl.dat", "resources.pak", "chrome_100_percent.pak", "chrome_200_percent.pak", "v8_context_snapshot.bin", "locales/en-US.pak"]
			.iter()
			.map(|f| (PathBuf::from(f), *f == "libcef.so"))
			.collect()),
		Platform::Windows => {
			let mut files: Vec<(PathBuf, bool)> = fs::read_dir(cef_dir)
				.map_err(|e| format!("{}: {e}", cef_dir.display()))?
				.flatten()
				.map(|e| PathBuf::from(e.file_name()))
				.filter(|f| f.extension().is_some_and(|x| ["dll", "dat", "pak", "bin", "json"].iter().any(|k| x.eq_ignore_ascii_case(k))))
				.map(|f| (f, false))
				.collect();
			files.push((PathBuf::from("locales/en-US.pak"), false));
			Ok(files)
		}
	}
}

fn copy(from: &Path, to: &Path) -> Result<(), String> {
	if let Some(dir) = to.parent() {
		fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
	}
	fs::copy(from, to).map_err(|e| format!("{} -> {}: {e}", from.display(), to.display()))?;
	Ok(())
}

/// Strip a Linux binary's symbols and debug info in place.
fn strip(path: &Path) -> Result<(), String> {
	crate::run(Command::new("strip").arg("--strip-unneeded").arg(path))
}

/// CEF's licence, from the distribution's archive beside its folder
/// (`<version>/cef_binary_*.tar.bz2`, member `<archive name>/LICENSE.txt`).
fn cef_license(cef_dir: &Path) -> Result<Vec<u8>, String> {
	let version_dir = cef_dir.parent().ok_or("CEF's folder has no parent")?;
	let tarball = fs::read_dir(version_dir)
		.map_err(|e| format!("{}: {e}", version_dir.display()))?
		.flatten()
		.map(|e| e.path())
		.find(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("cef_binary_") && n.to_string_lossy().ends_with(".tar.bz2")))
		.ok_or(format!("no cef_binary_*.tar.bz2 in {} for CEF's LICENSE.txt", version_dir.display()))?;
	let stem = tarball.file_name().unwrap().to_string_lossy().trim_end_matches(".tar.bz2").to_string();
	let output = Command::new("tar")
		.arg("-xjf")
		.arg(&tarball)
		.arg("-O")
		.arg(format!("{stem}/LICENSE.txt"))
		.output()
		.map_err(|e| format!("tar: {e}"))?;
	if !output.status.success() || output.stdout.is_empty() {
		return Err(format!("no LICENSE.txt in {}", tarball.display()));
	}
	Ok(output.stdout)
}

/// The notice for Steam's library.
fn steam_notice(steam: &SteamLibrary) -> String {
	format!(
		"{file} is Valve Corporation's Steamworks SDK redistributable library, from the\n\
		 steamworks-sys {version} crate on crates.io ({member}),\n\
		 SHA-256 {sha256}.\n\
		 It is redistributed under the Steamworks SDK Access Agreement:\n\
		 https://partner.steamgames.com/documentation/sdk_access_agreement\n\
		 bb loads it only on a Steam install of Battle Brothers (see `bb --no-steam`).\n",
		file = steam.pin.file_name,
		version = bb_steam::pinned::CRATE_VERSION,
		member = steam.pin.member,
		sha256 = steam.pin.sha256,
	)
}

/// Lay the package out in `dir` (emptied first).
pub fn lay_out(dir: &Path, platform: Platform, bb: &Path, cef_dir: &Path, steam: &SteamLibrary) -> Result<(), String> {
	let _ = fs::remove_dir_all(dir);
	fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
	let exe = dir.join(bb.file_name().ok_or("bb has no file name")?);
	copy(bb, &exe)?;
	if platform == Platform::Linux {
		strip(&exe)?;
	}
	for (file, stripped) in cef_files(cef_dir, platform)? {
		let to = dir.join(&file);
		copy(&cef_dir.join(&file), &to)?;
		if stripped && platform == Platform::Linux {
			strip(&to)?;
		}
	}
	copy(&steam.path, &dir.join(steam.pin.file_name))?;
	crate::steam_library::check_file(&dir.join(steam.pin.file_name), steam.pin.sha256)?;
	let notices = dir.join("licenses");
	copy(&cef_dir.join("CREDITS.html"), &notices.join("CEF-CREDITS.html"))?;
	fs::write(notices.join("CEF-LICENSE.txt"), cef_license(cef_dir)?).map_err(|e| e.to_string())?;
	fs::write(notices.join("STEAMWORKS.txt"), steam_notice(steam)).map_err(|e| e.to_string())?;
	Ok(())
}

/// Pack `<out>/<name>` as `<out>/<name>.tar.gz` (`.zip` for Windows, with a `tar` that
/// writes zips: Windows' own and macOS's do).
pub fn archive(out: &Path, name: &str, platform: Platform) -> Result<PathBuf, String> {
	let (archive, flags) = match platform {
		Platform::Linux => (out.join(format!("{name}.tar.gz")), "-czf"),
		Platform::Windows => (out.join(format!("{name}.zip")), "-acf"),
	};
	let _ = fs::remove_file(&archive);
	crate::run(Command::new("tar").arg(flags).arg(&archive).arg("-C").arg(out).arg(name))?;
	Ok(archive)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn platforms_by_target() {
		assert_eq!(Platform::of("x86_64-unknown-linux-gnu"), Ok(Platform::Linux));
		assert_eq!(Platform::of("x86_64-pc-windows-msvc"), Ok(Platform::Windows));
		assert!(Platform::of("aarch64-apple-darwin").is_err());
		assert_eq!(exe_name("x86_64-pc-windows-msvc"), "bb.exe");
		assert_eq!(exe_name("x86_64-unknown-linux-gnu"), "bb");
	}
}
