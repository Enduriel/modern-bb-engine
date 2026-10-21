//! The install's hardware cursors (`<install>/cursors/*.cur`) read as the original's:
//! 32×32 with alpha, at the hotspots their files give. Skips cleanly when the Steam
//! install isn't there.

use std::path::{Path, PathBuf};

use bb_assets::parse_cursor;

/// The install's `cursors`, beside its data directory (`$BB_DATA_DIR`, else the default
/// Linux Steam install, as `bb_engine::harness::STEAM_DATA_DIR`).
fn cursors() -> PathBuf {
	match std::env::var_os("BB_DATA_DIR") {
		Some(data) => Path::new(&data).join("../cursors"),
		None => Path::new(&std::env::var_os("HOME").unwrap_or_default()).join(".local/share/Steam/steamapps/common/Battle Brothers/cursors"),
	}
}

/// `what` is missing: fails the test unless `BB_ALLOW_SKIPS=1` lets it skip (as
/// `bb_engine::harness::skip`, which this crate can't use).
fn skip(what: &str) {
	if std::env::var_os("BB_ALLOW_SKIPS").is_some_and(|v| v == "1") {
		eprintln!("skipping: {what}");
	} else {
		panic!("this test can't run here: {what} (set BB_ALLOW_SKIPS=1 to skip it on a machine without it)");
	}
}

#[test]
fn the_installs_cursors_read() {
	let dir = &cursors();
	if !dir.exists() {
		skip(&format!("{} not present", dir.display()));
		return;
	}
	let read = |name: &str| parse_cursor(&std::fs::read(dir.join(format!("{name}.cur"))).expect("file")).expect(name);
	for (name, hotspot) in [("cursor_hand", (1, 1)), ("cursor_boot", (2, 11)), ("cursor_scroll", (15, 15)), ("cursor_enter", (0, 0))] {
		let c = read(name);
		assert_eq!((c.width, c.height, c.hotspot), (32, 32, hotspot), "{name}");
		assert!(c.rgba.chunks_exact(4).any(|p| p[3] == 255) && c.rgba.chunks_exact(4).any(|p| p[3] == 0), "{name}: a shape on clear");
	}
	let all = std::fs::read_dir(dir).expect("dir").flatten().filter(|e| e.path().extension().is_some_and(|x| x == "cur")).count();
	assert_eq!(all, 12, "the twelve cursors (no cursor_rotate)");
}
