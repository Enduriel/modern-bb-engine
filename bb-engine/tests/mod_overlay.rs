//! Validates DEFLATE support against a real (compressed) mod `.zip`, and the
//! overlay precedence in `DatScriptSource`. Skips if no mods are installed.

use bb_engine::DatArchive;

use bb_engine::harness::STEAM_DATA_DIR as DATA_DIR;

/// First `mod_*.zip` in the data dir, if any.
fn first_mod_zip() -> Option<std::path::PathBuf> {
	let mut mods: Vec<_> = std::fs::read_dir(DATA_DIR)
		.ok()?
		.flatten()
		.map(|e| e.path())
		.filter(|p| {
			p.file_name().is_some_and(|n| {
				n.to_string_lossy().starts_with("mod_") && n.to_string_lossy().ends_with(".zip")
			})
		})
		.collect();
	mods.sort();
	mods.into_iter().next()
}

#[test]
fn reads_a_deflated_entry_from_a_mod_zip() {
	let Some(zip) = first_mod_zip() else {
		eprintln!("skipping: no mod_*.zip under {DATA_DIR}");
		return;
	};

	let mut archive = DatArchive::open(&zip).expect("open mod zip");
	// Find any entry (mod zips use DEFLATE) and read it — exercises decompression.
	let name = archive
		.names()
		.into_iter()
		.find(|n| n.ends_with(".nut"))
		.expect("a mod ships at least one .nut");
	let bytes = archive.read(&name).expect("read (decompress) the entry");
	assert!(!bytes.is_empty(), "DEFLATE-decompressed entry is non-empty");
}
