//! Drive the REAL game to the main menu, headless, via the [`Engine`] host: load the
//! compiled scripts, boot `scripts/root_state` (which builds the global UI objects
//! and adds `MainMenuState`), and print the SQ→JS calls the menu made. The `Engine`
//! owns the VM + services and tears them down cleanly on drop.
//!
//! Run: `cargo run -p bb-engine --example boot_menu -- [data-dir]`  (LEAN=1 loads
//! only config/dlc and lazy-loads the rest — far fewer scripts.)

use std::path::PathBuf;

use bb_engine::DatScriptSource;
use squirrel::SqTable;
use squirrel_bb::Engine;

const DEFAULT_DATA: &str = concat!(env!("HOME"), "/.local/share/Steam/steamapps/common/Battle Brothers/data");

fn main() {
	let dir = PathBuf::from(
		std::env::args()
			.nth(1)
			.unwrap_or_else(|| DEFAULT_DATA.to_string()),
	);
	println!("booting menu against: {}", dir.display());

	let mut dats: Vec<PathBuf> = std::fs::read_dir(&dir)
		.expect("read data dir")
		.flatten()
		.map(|e| e.path())
		.filter(|p| {
			let name = p
				.file_name()
				.map(|n| n.to_string_lossy().into_owned())
				.unwrap_or_default();
			name.starts_with("data_") && name.ends_with(".dat")
		})
		.collect();
	dats.sort();
	let source = DatScriptSource::open(&dats).expect("open archives");

	let mut engine = Engine::new(source);
	engine.vm_mut().set_print_handler(|_| {});

	// Load config (+dlc); LEAN relies on `new` to lazy-load the rest.
	let lean = std::env::var("LEAN").is_ok();
	let sweep = format!(
		"::lean <- {lean};\n{}",
		r#"
		::Const <- {};
		::loaded <- 0; ::failed <- 0; ::errors <- [];
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
		foreach (f in ::IO.enumerateFiles("scripts/config/")) if (priority.find(f) == null) tryInclude(f);
		local dlc = ::IO.enumerateFiles("scripts/dlc/");
		if (dlc != null) foreach (f in dlc) tryInclude(f);
		if (!::lean) foreach (f in ::IO.enumerateFiles("scripts/")) if (f != "scripts/preload") tryInclude(f);
	"#
	);
	engine.run_script(&sweep, "sweep").expect("sweep");
	{
		let root = SqTable::root(engine.vm());
		println!(
			"\n=== load === {} loaded, {} failed",
			root.get::<i64>("loaded").unwrap_or(0),
			root.get::<i64>("failed").unwrap_or(0)
		);
	}

	println!("\n=== boot root_state ===");
	match engine.boot("scripts/root_state") {
		Ok(_) => println!("✅ booted; states: {:?}", engine.root_state().state_names()),
		Err(e) => println!("❌ boot failed:\n   {e}"),
	}

	// Each recorded call is an SQ→JS instruction the (headless) menu issued.
	let calls = engine.ui().calls();
	println!("\n=== UI calls recorded: {} ===", calls.len());
	for c in &calls {
		let scope = match &c.module {
			Some(m) => format!("{}/{}", c.screen, m),
			None => c.screen.clone(),
		};
		println!(
			"  {}{} .{}",
			if c.sync { "[sync] " } else { "" },
			scope,
			c.method
		);
	}
	drop(calls); // SqStrongRef snapshot — drop before the engine (VM) tears down.

	// `engine` drops here: clears all services, then closes the VM cleanly.
}
