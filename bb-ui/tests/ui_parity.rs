//! UI parity with the original: our page, driven through the main menu,
//! the new-campaign page and the Options screen, matches the original's snapshots
//! (`tests/goldens/`, captured with `probes/ui_goldens.js`) in structure, text, boxes
//! and the compared styles, apart from the known differences below. Skips if there is
//! no Steam install.
//!
//! Run: `cargo test -p bb-ui --features cef --test ui_parity`
//!
//! A plain `main` (`harness = false`), like `menu_ui`: Chromium re-runs this binary for
//! its child processes, and CEF can only start once per process.

use std::path::Path;
use std::process::ExitCode;

use bb_engine::harness::STEAM_DATA_DIR;
use bb_ui::host::{HostConfig, UiSession, goldens};
use bb_ui::parity::{Difference, compare};

/// Differences we understand and haven't fixed yet.
fn known(screen: &str, difference: &Difference) -> bool {
	match difference {
		// The base font size: `body` is 62.5% of the root's 10px. Coherent computes 8px
		// (a minimum), Chromium 6.25px, and elements that set no size inherit it. No box
		// differs because of it on these screens.
		Difference::Style {
			property,
			golden,
			ours,
			..
		} if property == "font-size" && golden == "8px" && ours == "6.25px" => true,
		// The origin list's horizontal scrollbar: an empty, zero-height row 10px wider
		// than the list counts toward its scroll width in Chromium but didn't in old
		// WebKit, so aciScrollBar adds the bar (and the list gets 32px shorter).
		_ => {
			screen == "new_campaign"
				&& difference
					.path()
					.contains("div.l-list-container[1] > div.ui-control.list.aciScrollBar")
		}
	}
}

fn matches_the_originals_screens(data_dir: &Path) -> Result<(), String> {
	let dir = goldens::default_dir();
	let (width, height) = goldens::load(&dir, goldens::SCREENS[0].0)?.viewport;
	let config = HostConfig {
		width: width as u32,
		height: height as u32,
		..HostConfig::default()
	};
	let session = UiSession::start(data_dir, config, false)?;
	session.boot_menu()?;

	let mut unexpected = Vec::new();
	for (name, ours) in goldens::capture(&session)? {
		let golden = goldens::load(&dir, name)?;
		for difference in compare(&golden, &ours, &goldens::tolerance(name)) {
			if !known(name, &difference) {
				unexpected.push(format!("{name}: {difference}"));
			}
		}
	}
	if unexpected.is_empty() {
		Ok(())
	} else {
		Err(format!(
			"{} unexpected differences:\n{}",
			unexpected.len(),
			unexpected.join("\n")
		))
	}
}

fn main() -> ExitCode {
	if let Some(code) = bb_ui::host::run_subprocess() {
		return ExitCode::from(code.clamp(0, 255) as u8);
	}
	let name = "matches_the_originals_screens";
	let data_dir = Path::new(STEAM_DATA_DIR);
	if !data_dir.exists() {
		println!(
			"test {name} ... skipped ({} not present)",
			data_dir.display()
		);
		return ExitCode::SUCCESS;
	}
	match matches_the_originals_screens(data_dir) {
		Ok(()) => {
			println!("test {name} ... ok");
			ExitCode::SUCCESS
		}
		Err(error) => {
			println!("test {name} ... FAILED\n{error}");
			ExitCode::FAILURE
		}
	}
}
