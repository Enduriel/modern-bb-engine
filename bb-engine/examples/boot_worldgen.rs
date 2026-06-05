//! Drive the REAL world generator headless: boot to the menu (which builds MapGen +
//! loads all map templates), then `World.resizeScene(140,140)` + run the worldmap
//! generator's `fill(...)`. Reports how far generation gets, and the first
//! unimplemented World native it hits, if any. (This checks the grid structure and the
//! native API; seed-exactness is `tests/world_boot.rs`'s.)
//!
//! Run: `cargo run -p bb-engine --example boot_worldgen -- [data-dir]`

use std::path::PathBuf;

use bb_engine::DatScriptSource;
use squirrel_bb::Engine;

const DEFAULT_DATA: &str = concat!(env!("HOME"), "/.local/share/Steam/steamapps/common/Battle Brothers/data");

fn main() {
	let dir = PathBuf::from(
		std::env::args()
			.nth(1)
			.unwrap_or_else(|| DEFAULT_DATA.to_string()),
	);
	let mut dats: Vec<PathBuf> = std::fs::read_dir(&dir)
		.expect("read data dir")
		.flatten()
		.map(|e| e.path())
		.filter(|p| {
			let n = p
				.file_name()
				.map(|n| n.to_string_lossy().into_owned())
				.unwrap_or_default();
			n.starts_with("data_") && n.ends_with(".dat")
		})
		.collect();
	dats.sort();
	let source = DatScriptSource::open(&dats).expect("open archives");

	let mut engine = Engine::new(source);
	// Route prints (incl. the error call stack) to stderr, and install the stdlib
	// error handlers so an unhandled script error prints a full call stack.
	engine.vm_mut().set_print_handler(|s| eprint!("{s}"));
	engine.vm_mut().install_standard_error_handlers();

	// Load config (+dlc); MapGen lazy-loads the templates during boot.
	engine
		.run_script(
			r#"
			::Const <- {};
			local function inc(f) { try { ::include(f); } catch(e){} }
			inc("scripts/tools/weak_table_ref");
			foreach (g in ["scripts/config/global","scripts/config/ui","scripts/config/strings",
			               "scripts/config/character_names","scripts/config/spawnlist_master"]) inc(g);
			foreach (f in ::IO.enumerateFiles("scripts/config/")) inc(f);
			local dlc = ::IO.enumerateFiles("scripts/dlc/");
			if (dlc != null) foreach (f in dlc) inc(f);
			"#,
			"config",
		)
		.expect("config");
	engine
		.boot("scripts/root_state")
		.expect("boot menu (builds MapGen)");
	println!("menu booted; driving worldgen…");

	// Drive the world generator. Errors here name the next missing World native.
	let result = engine.run_script(
		r#"
		this.Math.seedRandomString("rust-engine-test");
		this.World.resizeScene(140, 140);
		// world_state normally stands up the entity manager + world state before worldgen.
		this.World.EntityManager <- this.new("scripts/entity/world/entity_manager");
		this.World.State <- { m = { Regions = [] } };
		local wg = this.MapGen.get("world.worldmap_generator");
		wg.fill({ X = 0, Y = 0, W = 140, H = 140 }, null);
		::genOk <- true;
		"#,
		"worldgen",
	);
	match result {
		Ok(()) => {
			println!("✅ worldgen.fill() ran to completion");
			let (w, h) = engine.world_grid().dims();
			println!(
				"   grid {w}x{h}; tile(70,70).Type = {:?}",
				engine.world_grid().tile_type(70, 70)
			);
		}
		Err(e) => println!("⏹  worldgen stopped at the next gap:\n   {e}"),
	}
}
