//! Which archive (the game's data or a mod) each script came from, by its source name.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::DatArchive;

/// The owner of the scripts nobody mounted: compiled from a string at run time
/// (`compilestring`) or the engine's own.
pub const UNKNOWN: &str = "(not from an archive)";
pub const NATIVE: &str = "(engine natives)";
pub const VANILLA: &str = "vanilla (data_*.dat)";

pub struct Owners {
	/// Script path without its extension → the archive that provides it (the last
	/// mounted, as the overlay resolves it).
	by_path: HashMap<String, String>,
}

impl Owners {
	/// The owners of the scripts in `archives` (in mount order).
	pub fn of(archives: &[PathBuf]) -> Self {
		let mut by_path = HashMap::new();
		for path in archives {
			let file = path.file_name().unwrap().to_string_lossy().into_owned();
			let owner = if file.starts_with("data_") && file.ends_with(".dat") { VANILLA.to_string() } else { file };
			let Ok(archive) = DatArchive::open(crate::harness::locate_mod(path)) else { continue };
			for name in archive.names() {
				if let Some(stem) = name.strip_suffix(".cnut").or_else(|| name.strip_suffix(".nut")) {
					by_path.insert(stem.to_ascii_lowercase(), owner.clone());
				}
			}
		}
		Owners { by_path }
	}

	/// The owner of a script compiled from `source`.
	pub fn of_source<'a>(&'a self, source: Option<&str>) -> &'a str {
		let Some(source) = source else { return UNKNOWN };
		let stem = source.strip_suffix(".cnut").or_else(|| source.strip_suffix(".nut")).unwrap_or(source);
		self.by_path.get(&stem.to_ascii_lowercase()).map_or(UNKNOWN, String::as_str)
	}
}
