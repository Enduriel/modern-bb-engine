//! The compile cache for mod `.nut` scripts, through the real module system: a warm
//! run loads the cached bytecode and behaves exactly like the cold run that compiled
//! the source.
//! - A script whose compile declares constants (`const`, `enum`) is never cached, so
//!   the constant table ends up the same as in a cold run.
//! - A script compiled against constants is keyed on them.
//! - Errors from cached bytecode report the same source names and lines.

use std::cell::{Cell, RefCell};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use bb_engine::{DatScriptSource, ScriptCache};
use squirrel_bb::{Engine, LoadedScript, NativeConfig, ScriptSource};

const SCRIPTS: [(&str, &str); 3] = [
	(
		"scripts/cc/defines.nut",
		"const GREETING = \"hi\";\nenum Colour { Red, Green = 5 }\n::Defined <- true;\n",
	),
	(
		"scripts/cc/uses.nut",
		"::Used <- GREETING + Colour.Green;\n",
	),
	(
		"scripts/cc/plain.nut",
		"::Plain <- 41 + 1;\n::Fail <- function() {\n\tlocal a = null;\n\treturn a.missing;\n};\n",
	),
];

fn write_mod(dir: &Path, scripts: &[(&str, &str)]) -> PathBuf {
	let path = dir.join("mod_compile_cache_test.zip");
	let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
	let options =
		zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
	for (name, body) in scripts {
		zip.start_file(*name, options).unwrap();
		zip.write_all(body.as_bytes()).unwrap();
	}
	zip.finish().unwrap();
	path
}

/// A `DatScriptSource` that counts how its `.nut`s were answered.
struct Counting {
	inner: DatScriptSource,
	source: Rc<Cell<usize>>,
	cached: Rc<Cell<usize>>,
}

impl ScriptSource for Counting {
	fn load(&self, path: &str) -> Option<LoadedScript> {
		self.inner.load(path)
	}
	fn list_prefix(&self, prefix: &str) -> Option<Vec<String>> {
		self.inner.list_prefix(prefix)
	}
	fn read_file(&self, path: &str) -> Option<Vec<u8>> {
		self.inner.read_file(path)
	}
	fn caches_compiles(&self) -> bool {
		self.inner.caches_compiles()
	}
	fn load_for(&self, path: &str, context: &dyn Fn() -> u64) -> Option<LoadedScript> {
		let script = self.inner.load_for(path, context);
		match &script {
			Some(LoadedScript::Source(_)) => self.source.set(self.source.get() + 1),
			Some(LoadedScript::Bytecode(_)) => self.cached.set(self.cached.get() + 1),
			None => {}
		}
		script
	}
	fn compiled(&self, path: &str, context: u64, bytecode: &[u8]) {
		self.inner.compiled(path, context, bytecode);
	}
}

struct Run {
	used: String,
	plain: i64,
	error_report: String,
	from_source: usize,
	from_cache: usize,
}

fn run(zip: &Path, cache_root: &Path) -> Run {
	let (source, cached) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
	let counting = Counting {
		inner: DatScriptSource::open([zip])
			.unwrap()
			.with_cache(Some(ScriptCache::at(cache_root))),
		source: source.clone(),
		cached: cached.clone(),
	};
	let mut engine = Engine::with_config(counting, &NativeConfig::default());
	let report = Rc::new(RefCell::new(String::new()));
	let (out, err) = (report.clone(), report.clone());
	engine
		.vm_mut()
		.set_print_handler(move |s| out.borrow_mut().push_str(s));
	engine
		.vm_mut()
		.set_error_handler(move |s| err.borrow_mut().push_str(s));
	engine.vm_mut().install_standard_error_handlers();
	engine
		.run_script(
			"::include(\"scripts/cc/defines\"); ::include(\"scripts/cc/uses\"); ::include(\"scripts/cc/plain\");",
			"load",
		)
		.expect("the scripts load");
	report.borrow_mut().clear();
	assert!(engine.run_script("::Fail();", "fail").is_err());
	let error_report = report.borrow().clone();
	let root = squirrel::SqTable::root(engine.vm());
	Run {
		used: root.get::<String>("Used").expect("Used"),
		plain: root.get::<i64>("Plain").expect("Plain"),
		error_report,
		from_source: source.get(),
		from_cache: cached.get(),
	}
}

#[test]
fn warm_runs_load_cached_bytecode_and_behave_identically() {
	let dir = std::env::temp_dir().join(format!("bb-compile-cache-it-{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).unwrap();
	let zip = write_mod(&dir, &SCRIPTS);
	let cache_root = dir.join("cache");

	let cold = run(&zip, &cache_root);
	assert_eq!((cold.used.as_str(), cold.plain), ("hi5", 42));
	assert_eq!((cold.from_source, cold.from_cache), (3, 0));
	assert!(
		cold.error_report.contains("scripts/cc/plain") && cold.error_report.contains("line [4]"),
		"{}",
		cold.error_report
	);

	let warm = run(&zip, &cache_root);
	assert_eq!((warm.used.as_str(), warm.plain), ("hi5", 42));
	// `defines` declares constants, so it always compiles from source; the other two
	// come from the cache.
	assert_eq!((warm.from_source, warm.from_cache), (1, 2));
	assert_eq!(warm.error_report, cold.error_report);

	// Change the constants: `uses` was compiled against the old ones, so its cache entry
	// must not be used. The key covers the whole constant table (the cache cannot know
	// which constants a script reads), so `plain`, compiled after them, misses too.
	let changed = [
		(
			"scripts/cc/defines.nut",
			"const GREETING = \"yo\";\nenum Colour { Red, Green = 7 }\n",
		),
		SCRIPTS[1],
		SCRIPTS[2],
	];
	let zip = write_mod(&dir, &changed);
	let after = run(&zip, &cache_root);
	assert_eq!(after.used, "yo7");
	assert_eq!((after.from_source, after.from_cache), (3, 0));
	let again = run(&zip, &cache_root);
	assert_eq!(again.used, "yo7");
	assert_eq!((again.from_source, again.from_cache), (1, 2));
	let _ = std::fs::remove_dir_all(&dir);
}
