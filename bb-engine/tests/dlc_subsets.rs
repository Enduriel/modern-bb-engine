//! DLC compatibility: the game with any set of DLCs. Each paid DLC ships its init
//! script in an archive of its own (`data_002.dat` Fangshire … `data_010.dat` Of Flesh
//! and Faith), which Steam installs only for owners, and the engine takes "the script is
//! installed" as ownership (`installed_dlc`). With some DLC archives left out, the
//! scripts must see exactly the DLCs that are there (`::Const.DLC.<Name>` and its
//! `Mask`), the menu must boot and a new campaign must start and run. A save made
//! with a DLC the game lacks must read as incompatible (`::Const.DLC.isCompatible`).
//!
//! Skips if no Steam install.

use std::path::{Path, PathBuf};

use bb_engine::{harness, tactical_player as player};

/// Each DLC's archive and its `::Const.DLC` flag.
const DLCS: [(&str, &str); 6] = [
	("data_002.dat", "Fangshire"),
	("data_003.dat", "Lindwurm"),
	("data_004.dat", "Unhold"),
	("data_006.dat", "Wildmen"),
	("data_008.dat", "Desert"),
	("data_010.dat", "Paladins"),
];

fn eval(engine: &harness::GameEngine, expr: &str) -> i64 {
	player::eval_i64(engine, expr).unwrap_or_else(|e| panic!("{expr}: {e}")).unwrap_or_else(|| panic!("{expr}: no value"))
}

fn frames(engine: &harness::GameEngine, n: u32) {
	for _ in 0..n {
		harness::run_frame(engine, 1000.0 / 60.0).expect("frame");
	}
}

/// Boot the menu and a new campaign with only the DLC archives not in `missing`, and
/// return `::Const.DLC.Mask`. A save made with `full_mask` (every DLC) must read as
/// compatible exactly when no DLC it needs is missing.
fn check_without(dir: &Path, missing: &[&str], full_mask: Option<i64>) -> i64 {
	let archives: Vec<PathBuf> = harness::data_archives(dir)
		.expect("list the data archives")
		.into_iter()
		.filter(|p| !missing.iter().any(|m| p.file_name().is_some_and(|n| n == *m)))
		.collect();
	let engine = harness::open_mounted(dir, archives, |_| {}).expect("open the archives");
	harness::boot_menu(&engine).expect("boot the menu");
	let what = format!("without {missing:?}");

	// The scripts see exactly the DLCs that are installed.
	for (archive, flag) in DLCS {
		let owned = !missing.contains(&archive);
		let seen = eval(&engine, &format!("((\"{flag}\" in ::Const.DLC) && ::Const.DLC.{flag}) ? 1 : 0")) == 1;
		assert_eq!(seen, owned, "{what}: Const.DLC.{flag}");
	}
	let mask = eval(&engine, "::Const.DLC.Mask");

	// A new campaign starts and the world runs.
	harness::prepare_new_campaign(&engine, "rust-engine-test").unwrap_or_else(|e| panic!("{what}: prepare: {e}"));
	harness::start_new_campaign(&engine).unwrap_or_else(|e| panic!("{what}: startNewCampaign: {e}"));
	frames(&engine, 60);
	engine
		.run_script("::World.State.setPause(false);", "unpause")
		.unwrap_or_else(|e| panic!("{what}: unpause: {e}"));
	frames(&engine, 300);
	assert!(eval(&engine, "::World.getPlayerRoster().getSize()") > 0, "{what}: no company");

	// A save made with every DLC reads as incompatible when one it needs is missing.
	if let Some(full_mask) = full_mask {
		let compatible = eval(
			&engine,
			&format!(
				"(function() {{ local meta = {{ getInt = function(_k) {{ return _k == \"dlc\" ? {full_mask} : 0; }} }}; return ::Const.DLC.isCompatible(meta) ? 1 : 0; }})()"
			),
		);
		assert_eq!(compatible == 1, full_mask & !mask == 0, "{what}: a full-DLC save's compatibility (mask {mask:#x} of {full_mask:#x})");
	}
	mask
}

#[test]
fn every_dlc_subset_boots_and_starts_a_campaign() {
	let dir = Path::new(harness::STEAM_DATA_DIR);
	if !dir.exists() {
		eprintln!("skipping: {} not present", dir.display());
		return;
	}
	let all: Vec<&str> = DLCS.iter().map(|d| d.0).collect();
	// The full set, the base game alone, and each DLC missing on its own.
	let full_mask = check_without(dir, &[], None);
	assert_eq!(check_without(dir, &all, Some(full_mask)) & full_mask, 0, "the base game's mask has DLC bits");
	for dlc in &all {
		check_without(dir, &[dlc], Some(full_mask));
	}
}
