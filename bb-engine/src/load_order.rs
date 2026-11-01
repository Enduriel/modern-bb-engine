//! The order the original mounts its archives in: every `data_*.dat` and mod `.zip` in
//! `data/` together, sorted by file name, byte by byte (probed 2026-11-01: `aaa.zip` and
//! `Zlt.zip` load before vanilla's `data_*.dat`, whose files replace theirs, and
//! `mod_lt.zip` after). A later archive overrides an earlier one.

use std::path::Path;

/// Sort `archives` into load order, by file name (a stable sort: two of the same name
/// keep their order).
pub fn sort<P: AsRef<Path>>(archives: &mut [P]) {
	archives.sort_by(|a, b| a.as_ref().file_name().cmp(&b.as_ref().file_name()));
}

#[cfg(test)]
mod tests {
	use std::path::PathBuf;

	#[test]
	fn archives_sort_together_by_file_name_byte_by_byte() {
		let mut archives: Vec<PathBuf> =
			["/d/data_002.dat", "/m/mod_b.zip", "/d/data_001.dat", "/m/Zlt.zip", "/m/aaa.zip", "/m/mod_a.zip"]
				.map(PathBuf::from)
				.into();
		super::sort(&mut archives);
		let names: Vec<_> = archives.iter().map(|p| p.file_name().unwrap().to_str().unwrap()).collect();
		// Upper case sorts before lower case: `Zlt.zip` loads before vanilla.
		assert_eq!(names, ["Zlt.zip", "aaa.zip", "data_001.dat", "data_002.dat", "mod_a.zip", "mod_b.zip"]);
	}
}
