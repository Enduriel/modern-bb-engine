//! Boot harness / discovery tool: point a file-system [`ScriptSource`] at a
//! Battle Brothers `scripts/` tree, wire up the natives + module system + Time
//! service, and attempt `preload`.
//!
//! This is NOT a finished host — it is how we exercise real vanilla scripts to
//! find the next missing engine native. It is expected to fail partway through
//! (the World/Tactical APIs don't exist yet); the failure point is the signal.
//!
//! Run: `cargo run -p squirrel-bb --example boot -- [path-to-vanilla]`
//! (defaults to `../vanilla`: a folder of the game's unpacked scripts beside the
//! workspace).

use std::path::{Path, PathBuf};

use squirrel::{SQVM, SqArray, SqTable};
use squirrel_bb::{LoadedScript, ModuleSystem, ScriptSource, TimeService, install_natives};

/// A `ScriptSource` backed by real files under `root`: `path` maps to
/// `<root>/<path>.nut`.
struct FileScriptSource {
	root: PathBuf,
}

impl ScriptSource for FileScriptSource {
	fn load(&self, path: &str) -> Option<LoadedScript> {
		std::fs::read(self.root.join(format!("{path}.nut")))
			.ok()
			.map(LoadedScript::Source)
	}

	fn list_prefix(&self, prefix: &str) -> Option<Vec<String>> {
		let dir = self.root.join(prefix.trim_end_matches('/'));
		if !dir.is_dir() {
			return None; // missing directory -> null (preload's DLC sweep guards on this)
		}
		let mut paths = Vec::new();
		collect_nut_files(&self.root, &dir, &mut paths);
		paths.sort();
		Some(paths)
	}
}

/// Recursively collect `.nut` files under `dir` as root-relative, extension-less,
/// forward-slash paths.
fn collect_nut_files(root: &Path, dir: &Path, out: &mut Vec<String>) {
	let Ok(entries) = std::fs::read_dir(dir) else {
		return;
	};
	for entry in entries.flatten() {
		let path = entry.path();
		if path.is_dir() {
			collect_nut_files(root, &path, out);
		} else if path.extension().is_some_and(|ext| ext == "nut") {
			if let Ok(rel) = path.strip_prefix(root) {
				out.push(rel.with_extension("").to_string_lossy().replace('\\', "/"));
			}
		}
	}
}

fn main() {
	let root = std::env::args()
		.nth(1)
		.unwrap_or_else(|| "../vanilla".to_string());
	let root = PathBuf::from(&root);
	println!("booting against script root: {}", root.display());

	let mut vm = SQVM::new(1 << 16);
	// Swallow per-script error/compile chatter; we tally failures ourselves below.
	vm.set_error_handler(|_| {});
	vm.set_compile_error_handler(|_| {});
	vm.set_print_handler(|_| {});

	install_natives(&mut vm);
	let modules = ModuleSystem::new(FileScriptSource { root });
	modules.install(&mut vm);
	let time = TimeService::new();
	time.install(&mut vm);

	// Error-tolerant sweep that follows preload.nut's ORDER (priority config, then
	// the rest of config, then DLC, then everything) but try/catches each include
	// so one bad file doesn't stop the run. Config must load first — class scripts
	// read this.Const.* at definition time.
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
		// Skip preload itself: re-running it would reset ::Const (its first line is
		// `Const <- {}`), and the already-loaded config can't repopulate it. In the
		// real game the host enters via preload, so its self-include is a no-op.
		foreach (f in ::IO.enumerateFiles("scripts/")) {
			if (f != "scripts/preload") tryInclude(f);
		}
	"#;

	if let Err(err) = vm.run_script(sweep, "discovery-sweep") {
		println!("\n❌ sweep harness itself failed: {err}");
		return;
	}

	let root_table = SqTable::root(&vm);
	let loaded = root_table.get::<i64>("loaded").unwrap_or(0);
	let failed = root_table.get::<i64>("failed").unwrap_or(0);
	println!("\n=== load sweep ===");
	println!("included OK: {loaded}");
	println!("failed:      {failed}");
	if let Some(errors) = root_table.get::<SqArray>("errors") {
		println!("\n--- failures ---");
		for entry in errors.to_vec::<String>() {
			println!("{entry}");
		}
	}
}
