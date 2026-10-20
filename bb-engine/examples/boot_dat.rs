//! Boot harness over the REAL game archives: decrypt + transcode + run the actual
//! compiled `.cnut` scripts from the installed `.dat` files.
//!
//! Like the squirrel-bb `boot` example but sourced from `DatScriptSource` instead
//! of decompiled `.nut` text. Expected to fail partway (World/Tactical runtime
//! natives don't exist yet); the failure points are the signal, and reaching the
//! config sweep at all exercises float/string transcoding on real data.
//!
//! Run: `cargo run -p bb-engine --example boot_dat -- [path-to-data-dir]`
//! (defaults to the Steam install).

use std::path::PathBuf;

use bb_engine::DatScriptSource;
use squirrel::{SQVM, SqArray, SqTable};
use squirrel_bb::{ModuleSystem, TimeService, install_natives};

use bb_engine::harness::STEAM_DATA_DIR as DEFAULT_DATA;

fn main() {
	let dir = PathBuf::from(
		std::env::args()
			.nth(1)
			.unwrap_or_else(|| DEFAULT_DATA.to_string()),
	);
	println!("booting against archive dir: {}", dir.display());

	// Archives in BB load order (sorted by filename; later overrides earlier).
	// By default just the base + DLC `data_*.dat`; pass `--mods` to also overlay
	// the `mod_*.zip`s (the real, modded game — useful but noisier).
	let with_mods = std::env::args().any(|a| a == "--mods");
	let mut dats: Vec<PathBuf> = std::fs::read_dir(&dir)
		.expect("read data dir")
		.flatten()
		.map(|e| e.path())
		.filter(|p| {
			let name = p
				.file_name()
				.map(|n| n.to_string_lossy().into_owned())
				.unwrap_or_default();
			(name.starts_with("data_") && name.ends_with(".dat"))
				|| (with_mods && name.starts_with("mod_") && name.ends_with(".zip"))
		})
		.collect();
	dats.sort();
	println!(
		"archives: {}",
		dats.iter()
			.map(|p| p.file_name().unwrap().to_string_lossy())
			.collect::<Vec<_>>()
			.join(", ")
	);

	let source = DatScriptSource::open(&dats).expect("open archives");

	let mut vm = SQVM::new(1 << 16);
	vm.set_error_handler(|_| {});
	vm.set_compile_error_handler(|_| {});
	vm.set_print_handler(|_| {});

	install_natives(&mut vm);
	let modules = ModuleSystem::new(source);
	modules.install(&mut vm);
	let time = TimeService::new();
	time.install(&mut vm);

	// Error-tolerant preload-order sweep (config first; skip re-running preload).
	let sweep = r#"
		::Const <- {};
		::loaded <- 0;
		::failed <- 0;
		::errors <- [];
		local function tryInclude(f) {
			try { ::include(f); ::loaded = ::loaded + 1; }
			catch (e) { ::failed = ::failed + 1; ::errors.push(f + " :: " + e); }
		}
		local priority = [
			"scripts/config/global", "scripts/config/ui", "scripts/config/strings",
			"scripts/config/character_names", "scripts/config/spawnlist_master"
		];
		tryInclude("scripts/tools/weak_table_ref");
		foreach (f in priority) tryInclude(f);
		foreach (f in ::IO.enumerateFiles("scripts/config/")) {
			if (priority.find(f) == null) tryInclude(f);
		}
		local dlc = ::IO.enumerateFiles("scripts/dlc/");
		if (dlc != null) foreach (f in dlc) tryInclude(f);
		foreach (f in ::IO.enumerateFiles("scripts/")) {
			if (f != "scripts/preload") tryInclude(f);
		}
	"#;

	if let Err(err) = vm.run_script(sweep, "dat-discovery-sweep") {
		println!("\n❌ sweep harness itself failed: {err}");
		return;
	}

	let root = SqTable::root(&vm);
	println!("\n=== load sweep (real .dat) ===");
	println!("included OK: {}", root.get::<i64>("loaded").unwrap_or(0));
	println!("failed:      {}", root.get::<i64>("failed").unwrap_or(0));
	if let Some(errors) = root.get::<SqArray>("errors") {
		println!("\n--- load failures ---");
		for entry in errors.to_vec::<String>() {
			println!("{entry}");
		}
	}

	// Phase 2 — run PAST load: try to instantiate representative game objects via
	// `this.new`, which runs their create() chains and hits the runtime natives
	// (World/Tactical/UI/...) that don't exist yet. Each probe is its own
	// run_script so a hard failure in one doesn't abort the rest.
	let targets = [
		("skill", "scripts/skills/actives/bash"),
		("skill container", "scripts/skills/skill_container"),
		("item container", "scripts/items/item_container"),
		("item", "scripts/items/weapons/arming_sword"),
		("tactical AI", "scripts/ai/tactical/agent"),
		("tactical entity", "scripts/entity/tactical/player"),
		("world entity", "scripts/entity/world/player_party"),
		("worldgen", "scripts/mapgen/map_generator"),
		("UI state", "scripts/states/main_menu_state"),
		("top-level state", "scripts/root_state"),
	];
	println!("\n=== run past load (this.new probes) ===");
	for (label, path) in targets {
		let script = format!(r#"this.new("{path}");"#);
		match vm.run_script(&script, "probe") {
			Ok(()) => println!("OK    {label:<16} {path}"),
			Err(e) => println!("FAIL  {label:<16} {path}\n        {e}"),
		}
	}
}
