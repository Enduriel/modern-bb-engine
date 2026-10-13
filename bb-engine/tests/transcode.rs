//! End-to-end: a real encrypted 3.0.4 `.cnut` from the installed game →
//! decrypt + transcode → load + run in our 64-bit Squirrel 3.1 VM.
//! Skips cleanly if the Steam install isn't present.

use bb_engine::{DatArchive, transcode_cnut};
use squirrel::SQVM;

const DATA_001: &str =
	concat!(env!("HOME"), "/.local/share/Steam/steamapps/common/Battle Brothers/data/data_001.dat");

#[test]
fn transcodes_and_runs_a_real_compiled_script() {
	if !std::path::Path::new(DATA_001).exists() {
		eprintln!("skipping: {DATA_001} not present");
		return;
	}

	// `weak_table_ref` is self-contained: it only uses getroottable + a `class`
	// definition + `.weakref()` — no engine natives, no Const — so a bare VM runs
	// it. (It's also preload.nut's very first include.)
	let mut archive = DatArchive::open(DATA_001).expect("open data_001.dat");
	let cnut = archive
		.read("scripts/tools/weak_table_ref.cnut")
		.expect("read weak_table_ref.cnut");

	let bytecode = transcode_cnut(&cnut).expect("transcode 3.0.4 -> our format");

	let vm = SQVM::new(1024);
	let closure = vm
		.load_closure(&bytecode)
		.expect("load the transcoded closure");
	// Running the main closure executes the file's top level, which defines the
	// WeakTableRef class on the root table.
	closure.call(&vm, ()).expect("run the transcoded script");

	// The class exists and actually works: its constructor + a method run.
	vm.run_script(
		r#"
		assert("WeakTableRef" in this);
		assert(typeof WeakTableRef == "class");
		local w = WeakTableRef(null);   // constructor: null -> WeakTable stays null
		assert(w.isNull());             // method call on the instance
		// The constructor assigns its own member, so `this` must be the new instance
		// (an unbound closure; see the transcoder's closure-environment rewrite).
		local t = { A = 1 };
		local w2 = WeakTableRef(t);
		assert(!w2.isNull());
		assert(w2.get().A == 1);
		assert(w2.A == 1);              // through the class's `_get`
		"#,
		"weak_table_ref_check",
	)
	.expect("the transcoded class behaves correctly");
}

#[test]
fn transcodes_and_loads_every_cnut_in_data_001() {
	if !std::path::Path::new(DATA_001).exists() {
		eprintln!("skipping: {DATA_001} not present");
		return;
	}

	let mut archive = DatArchive::open(DATA_001).expect("open data_001.dat");
	let cnut_names: Vec<String> = archive
		.names()
		.into_iter()
		.filter(|n| n.ends_with(".cnut"))
		.collect();
	assert!(
		cnut_names.len() > 3000,
		"expected ~3071 cnut files, got {}",
		cnut_names.len()
	);

	let vm = SQVM::new(2048);
	let mut failures: Vec<(String, String)> = Vec::new();
	for name in &cnut_names {
		let Some(cnut) = archive.read(name) else {
			failures.push((name.clone(), "read failed".into()));
			continue;
		};
		match transcode_cnut(&cnut).and_then(|bc| vm.load_closure(&bc).map_err(|e| e)) {
			Ok(_closure) => {} // dropped here, releasing it
			Err(e) => failures.push((name.clone(), e)),
		}
	}

	eprintln!(
		"transcoded + loaded {}/{} cnut files",
		cnut_names.len() - failures.len(),
		cnut_names.len()
	);
	for (name, err) in failures.iter().take(20) {
		eprintln!("  FAIL {name}: {err}");
	}
	assert!(
		failures.is_empty(),
		"{} cnut files failed to transcode/load",
		failures.len()
	);
}
