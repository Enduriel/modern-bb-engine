//! The UI parity goldens: which screens they show, how our page gets to
//! each one, and what their comparison leaves out. Captured from the original with
//! `bb-ui/probes/ui_goldens.js`, which walks the same path.

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::UiSession;
use crate::json::Json;
use crate::parity::{Snapshot, Tolerance};

/// The screens, in capture order, each with the clicks that lead to it from the
/// previous one.
pub const SCREENS: [(&str, &[&str]); 3] = [
	("main_menu", &[]),
	("new_campaign", &["New Campaign"]),
	("options", &["Cancel", "Options"]),
];

/// Where the goldens are kept in the repo.
pub fn default_dir() -> PathBuf {
	PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/goldens")
}

/// Read screen `name`'s golden from `dir`.
pub fn load(dir: &Path, name: &str) -> Result<Snapshot, String> {
	let path = dir.join(format!("{name}.json"));
	let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
	Snapshot::parse(&Json::parse(&text)?)
}

/// What screen `name`'s comparison leaves out: the Options resolution list shows the
/// machine's display modes (none on the probe's virtual display).
pub fn tolerance(name: &str) -> Tolerance {
	let mut tolerance = Tolerance::default();
	if name == "options" {
		tolerance
			.ignore_paths
			.push("div.control.l-list-container".into());
	}
	tolerance
}

/// Walk our page through [`SCREENS`] the way the golden probe does, snapshotting each
/// once it has settled.
pub fn capture(session: &UiSession) -> Result<Vec<(&'static str, Snapshot)>, String> {
	let step = Duration::from_secs(30);
	let mut snapshots = Vec::new();
	for (name, clicks) in SCREENS {
		for click in clicks {
			session.click_when_ready(click, step)?;
		}
		// As long as the probe waits: menus slide in, and lists get their scrollbars.
		session.run_frames(3.0)?;
		snapshots.push((name, session.snapshot()?));
	}
	Ok(snapshots)
}
