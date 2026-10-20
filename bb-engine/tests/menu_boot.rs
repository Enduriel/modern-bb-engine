//! End-to-end: the real game boots to a running main menu, headless, via the
//! [`Engine`] host. Loads config from the installed `.dat`s, boots
//! `scripts/root_state` (which lazy-loads the menu's deps), and asserts the menu
//! rendered itself via the recorded SQ→JS calls. The `Engine` tears down cleanly on
//! drop (the teardown-use-after-free regression guard). Skips if no Steam install.

use std::path::PathBuf;

use squirrel_bb::Engine;

use bb_engine::DatScriptSource;

use bb_engine::harness::STEAM_DATA_DIR as DATA_DIR;

#[test]
fn boots_to_a_running_main_menu_headless() {
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
	let source = DatScriptSource::open(&dats).expect("open archives");

	let mut engine = Engine::new(source);
	engine.vm_mut().set_print_handler(|_| {});

	// Load config + DLC; the menu's deps lazy-load via `new`.
	engine
		.run_script(
			r#"
			::Const <- {};
			local function inc(f) { ::include(f); }
			inc("scripts/tools/weak_table_ref");
			foreach (g in ["scripts/config/global","scripts/config/ui","scripts/config/strings",
			               "scripts/config/character_names","scripts/config/spawnlist_master"]) inc(g);
			foreach (f in ::IO.enumerateFiles("scripts/config/")) inc(f);
			local dlc = ::IO.enumerateFiles("scripts/dlc/");
			if (dlc != null) foreach (f in dlc) inc(f);
			"#,
			"menu-test-config",
		)
		.expect("config load");

	engine.boot("scripts/root_state").expect("boot the menu");

	// The main-menu state exists and ran its full lifecycle.
	assert_eq!(engine.root_state().state_names(), ["MainMenuState"]);

	// The menu told the UI to render itself (SQ→JS calls were recorded).
	let methods = engine.ui().methods_for("MainMenuScreen");
	assert!(
		methods.contains(&"show".to_string()),
		"MainMenuScreen.show (onShow ran)"
	);
	assert!(
		methods.contains(&"setVersion".to_string()),
		"MainMenuScreen.setVersion (onConnected ran)"
	);

	// The version payload is a scalar (a version string), not a table.
	let version = engine.ui().calls_to("MainMenuScreen", "setVersion");
	assert_eq!(version.len(), 1);
	assert!(version[0].as_table(engine.vm()).is_none());

	// `engine` drops here → clears all services, closes the VM cleanly. (No manual
	// per-service clear needed; that's the point of the host.)
}
