//! Float values in the real game config, through the transcode + f32 math path.
//! Regression for array-literal floats (`OP_APPENDARRAY` with `AAT_FLOAT`), which were
//! read as garbage f64s and made every world navigation cost impassable.
//! Skips cleanly when the Steam install isn't there.

use std::path::PathBuf;

use bb_engine::DatScriptSource;
use squirrel_bb::Engine;

use bb_engine::harness::STEAM_DATA_DIR as DATA_DIR;

#[test]
fn array_literal_floats_and_derived_costs_load_correctly() {
	if !std::path::Path::new(DATA_DIR).exists() {
		eprintln!("skipping: {DATA_DIR} not present");
		return;
	}
	let mut dats: Vec<PathBuf> = std::fs::read_dir(DATA_DIR)
		.expect("read data dir")
		.flatten()
		.map(|e| e.path())
		.filter(|p| {
			p.file_name().is_some_and(|n| {
				let n = n.to_string_lossy();
				n.starts_with("data_") && n.ends_with(".dat")
			})
		})
		.collect();
	dats.sort();
	let engine = Engine::new(DatScriptSource::open(&dats).expect("open archives"));
	engine
		.run_script(
			r#"
			::Const <- {};
			local function inc(f) { ::include(f); }
			inc("scripts/tools/weak_table_ref");
			foreach (g in ["scripts/config/global","scripts/config/ui","scripts/config/strings",
			               "scripts/config/character_names","scripts/config/spawnlist_master"]) inc(g);
			foreach (f in ::IO.enumerateFiles("scripts/config/")) inc(f);

			local close = @(a, b) (a - b) < 1e-6 && (b - a) < 1e-6;
			local speed = ::Const.World.TerrainTypeSpeedMult;
			// Array-literal floats (were garbage before the AAT_FLOAT hoist).
			assert(close(speed[2], 1.0) && close(speed[3], 0.5) && close(speed[4], 0.65));
			// Derived in game code: 10 * (1.0 / speed), computed in f32 mode.
			local nav = ::Const.World.TerrainTypeNavCost;
			assert(nav[0] == 0 && close(nav[2], 10.0) && close(nav[3], 20.0));
			"#,
			"config-floats",
		)
		.expect("config floats are correct");
}
