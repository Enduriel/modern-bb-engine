//! The install's hardware cursors (`<install>/cursors/*.cur`) read as the original's:
//! 32×32 with alpha, at the hotspots their files give. Skips cleanly when the Steam
//! install isn't there.

use std::path::{Path, PathBuf};

use bb_assets::parse_cursor;

/// The default Linux Steam install's `cursors`.
fn cursors() -> PathBuf {
	Path::new(&std::env::var_os("HOME").unwrap_or_default()).join(".local/share/Steam/steamapps/common/Battle Brothers/cursors")
}

#[test]
fn the_installs_cursors_read() {
	let dir = &cursors();
	if !dir.exists() {
		eprintln!("skipping: {} not present", dir.display());
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
