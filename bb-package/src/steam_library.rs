//! The target's Steam library, from the pinned `steamworks-sys` crate: fetched once
//! into the cache, and checked against the pins (`bb_steam::pinned`) every time it is
//! used, the crate and the library alike.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use bb_steam::pinned::{self, PinnedLibrary};

/// The checked library for a package.
pub struct SteamLibrary {
	pub path: PathBuf,
	pub pin: &'static PinnedLibrary,
}

/// Check the file at `path` against the `expected` SHA-256.
pub fn check_file(path: &Path, expected: &str) -> Result<(), String> {
	let actual = bb_steam::sha256::file_hex(path).map_err(|e| format!("{}: {e}", path.display()))?;
	pinned::verify(&path.display().to_string(), &actual, expected)
}

/// The pinned crate in `cache`, fetched from crates.io if it isn't there, and checked.
fn pinned_crate(cache: &Path) -> Result<PathBuf, String> {
	fs::create_dir_all(cache).map_err(|e| format!("{}: {e}", cache.display()))?;
	let path = cache.join(pinned::crate_file());
	if path.is_file() && check_file(&path, pinned::CRATE_SHA256).is_ok() {
		return Ok(path);
	}
	let part = cache.join(format!("{}.part", pinned::crate_file()));
	let url = pinned::crate_url();
	println!("fetching {url}");
	crate::run(Command::new("curl").args(["--fail", "--location", "--silent", "--show-error", "--proto", "=https", "--output"]).arg(&part).arg(&url))?;
	if let Err(e) = check_file(&part, pinned::CRATE_SHA256) {
		let _ = fs::remove_file(&part);
		return Err(e);
	}
	fs::rename(&part, &path).map_err(|e| format!("{}: {e}", path.display()))?;
	Ok(path)
}

/// Fetch (once) and check the Steam library for `target`.
pub fn fetch(target: &str, cache: &Path) -> Result<SteamLibrary, String> {
	let pin = pinned::library_for(target).ok_or(format!("no pinned Steam library for {target}"))?;
	let archive = pinned_crate(cache)?;
	let extract = cache.join(format!("steamworks-sys-{}", pinned::CRATE_VERSION));
	let path = extract.join(pin.member);
	if !path.is_file() || check_file(&path, pin.sha256).is_err() {
		let _ = fs::remove_dir_all(&extract);
		fs::create_dir_all(&extract).map_err(|e| format!("{}: {e}", extract.display()))?;
		crate::run(Command::new("tar").arg("-xzf").arg(&archive).arg("-C").arg(&extract).arg(pin.member))?;
	}
	check_file(&path, pin.sha256)?;
	Ok(SteamLibrary { path, pin })
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A file that isn't the pinned one is refused, whatever its name.
	#[test]
	fn a_changed_library_is_refused() {
		let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/tmp/bb-package-check");
		fs::create_dir_all(&dir).unwrap();
		let file = dir.join("libsteam_api.so");
		fs::write(&file, b"a library swapped in upstream").unwrap();
		let err = check_file(&file, pinned::LIBRARIES[0].sha256).unwrap_err();
		assert!(err.contains("refusing"), "{err}");
		let own = bb_steam::sha256::hex(b"a library swapped in upstream");
		assert!(check_file(&file, &own).is_ok());
	}
}
