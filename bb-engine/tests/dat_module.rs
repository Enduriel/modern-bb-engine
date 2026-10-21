//! The module system loading a real compiled script through `DatScriptSource`
//! (the bytecode `include` path): .cnut → decrypt → transcode → load_closure →
//! run. Gated on the installed game being present.

use bb_engine::DatScriptSource;
use squirrel::SQVM;
use squirrel_bb::{ModuleSystem, TimeService, install_natives};

/// The install's first archive (in `harness::STEAM_DATA_DIR`).
fn data_001() -> std::path::PathBuf {
	std::path::Path::new(bb_engine::harness::STEAM_DATA_DIR).join("data_001.dat")
}

#[test]
fn module_system_loads_a_compiled_script_from_a_dat() {
	if !data_001().exists() {
		bb_engine::harness::skip(format!("{} not present", data_001().display()));
		return;
	}

	// `vm` is declared first so it drops LAST — after `modules`/`time`, whose
	// strong refs must be released before the VM closes.
	let mut vm = SQVM::new(2048);
	let modules = ModuleSystem::new(DatScriptSource::open([data_001()]).expect("open data_001"));
	let time = TimeService::new();

	install_natives(&mut vm);
	modules.install(&mut vm);
	time.install(&mut vm);

	// include() pulls the compiled .cnut, transcodes it, and loads+runs it.
	modules
		.include(&vm, "scripts/tools/weak_table_ref")
		.expect("compiled script loads through the module system");

	vm.run_script(
		r#"
		assert("WeakTableRef" in this);
		assert(WeakTableRef(null).isNull());
		"#,
		"check",
	)
	.expect("the class from the compiled script works");

	drop(time);
	drop(modules);
}
