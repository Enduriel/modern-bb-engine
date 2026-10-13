//! The script cache must hand the VM exactly the bytes an uncached load does: for every
//! vanilla script, an uncached load, a load that fills a cold cache, and a load from
//! the warm cache all return identical bytecode. A damaged cache entry falls back to a
//! correct load. Skips if no Steam install.

use std::path::Path;

use bb_engine::harness::{STEAM_DATA_DIR, data_archives};
use bb_engine::{DatScriptSource, ScriptCache};
use squirrel_bb::{LoadedScript, ScriptSource};

fn bytes(script: Option<LoadedScript>) -> Vec<u8> {
	match script {
		Some(LoadedScript::Bytecode(b)) => b,
		Some(LoadedScript::Source(_)) => panic!("vanilla ships no source"),
		None => panic!("script failed to load"),
	}
}

#[test]
fn cached_loads_match_uncached_ones() {
	let Ok(paths) = data_archives(Path::new(STEAM_DATA_DIR)) else {
		eprintln!("skipping: no install at {STEAM_DATA_DIR}");
		return;
	};
	let root = std::env::temp_dir().join(format!("bb-script-cache-it-{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&root);
	let uncached = DatScriptSource::open(&paths).unwrap();
	let cached = DatScriptSource::open(&paths)
		.unwrap()
		.with_cache(Some(ScriptCache::at(&root)));
	let scripts = uncached.list_prefix("scripts/").unwrap();
	assert!(
		scripts.len() > 3000,
		"expected every vanilla script, got {}",
		scripts.len()
	);

	for s in &scripts {
		let expected = bytes(uncached.load(s));
		assert_eq!(bytes(cached.load(s)), expected, "{s}: cold cache");
		assert_eq!(bytes(cached.load(s)), expected, "{s}: warm cache");
	}

	// Damage every entry differently; each load must still be correct.
	let dir = ScriptCache::at(&root).dir().to_path_buf();
	for (i, file) in std::fs::read_dir(&dir).unwrap().flatten().enumerate() {
		let mut data = std::fs::read(file.path()).unwrap();
		match i % 3 {
			0 => data.truncate(data.len() / 2),
			1 => *data.last_mut().unwrap() ^= 0x40,
			_ => data[0] = b'X',
		}
		std::fs::write(file.path(), data).unwrap();
	}
	for s in scripts.iter().step_by(7) {
		assert_eq!(
			bytes(cached.load(s)),
			bytes(uncached.load(s)),
			"{s}: after damage"
		);
	}
	let _ = std::fs::remove_dir_all(&root);
}
