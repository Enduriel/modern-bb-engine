//! Mods require the modern engine as they require any mod: the engine registers itself
//! with Modern Hooks as `mod_modern_engine`, at its release version, before Modern Hooks
//! checks the mods' requirements (`docs/modding.md`). In the original game nothing
//! registers it, so Modern Hooks' popup names it as missing.
//!
//! Each test runs the game's preload (where Modern Hooks checks the requirements and runs
//! the queue) with Modern Hooks from the reference mod corpus (`bb_engine::mod_corpus`)
//! and a mod made for the test. Skips without the Steam install or the corpus.

use std::io::Write;
use std::path::{Path, PathBuf};

use bb_engine::{harness, tactical_player as player};

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// A Modern Hooks mod that requires `mod_modern_engine` by `requirement` (e.g.
/// `">= 0.4.0"`) and marks that its queue ran.
fn requiring_mod(dir: &Path, requirement: &str) -> PathBuf {
	let path = dir.join("mod_zz_requires_engine.zip");
	let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
	zip.start_file("scripts/!mods_preload/mod_zz_requires_engine.nut", zip::write::SimpleFileOptions::default())
		.unwrap();
	write!(
		zip,
		r#"local mod = ::Hooks.register("mod_zz_requires_engine", "1.0.0", "Requires the engine");
mod.require("mod_modern_engine {requirement}");
mod.queue(function() {{
	::bb_test_queue_ran <- true;
}});
"#
	)
	.unwrap();
	zip.finish().unwrap();
	path
}

/// The engine with the game's files, Modern Hooks and a mod requiring the engine by
/// `requirement`, after the game's preload; `None` when skipped.
fn preloaded(name: &str, requirement: &str) -> Option<harness::GameEngine> {
	let data = Path::new(harness::STEAM_DATA_DIR);
	if !data.exists() {
		harness::skip(format!("{} not present", harness::STEAM_DATA_DIR));
		return None;
	}
	let hooks = match bb_engine::mod_corpus::reference() {
		Ok(corpus) => corpus.find("mod_modern_hooks").expect("Modern Hooks in the reference mod corpus").clone(),
		Err(bb_engine::mod_corpus::CorpusError::Absent(why)) => {
			harness::skip(why);
			return None;
		}
		Err(e) => panic!("{e}"),
	};
	let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("engine_mod_requirement").join(name);
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).unwrap();
	let mut archives = harness::data_archives(data).unwrap();
	archives.push(hooks);
	archives.push(requiring_mod(&dir, requirement));
	let engine = harness::open_mounted(data, archives, |_| {}).expect("open with the mods");
	harness::load_config(&engine).expect("the game's preload");
	Some(engine)
}

fn eval(engine: &harness::GameEngine, expr: &str) -> String {
	player::eval_string(engine, expr).unwrap().unwrap_or_default()
}

/// What Modern Hooks' popup holds for the UI (it shows it once the UI connects).
fn popup_text(engine: &harness::GameEngine) -> String {
	eval(engine, "::Hooks.Popup.m.TextCache")
}

#[test]
fn the_engine_is_a_mod_that_meets_requirements() {
	let Some(engine) = preloaded("met", &format!(">= {VERSION}")) else {
		return;
	};
	assert_eq!(eval(&engine, r#"::Hooks.getMod("mod_modern_engine").getVersionString()"#), VERSION);
	assert_eq!(eval(&engine, r#"::Hooks.getMod("mod_modern_engine").getName()"#), "Modern Engine");
	assert_eq!(
		eval(
			&engine,
			&format!(r#"(::Hooks.getMod("mod_modern_engine").getVersion() >= ::Hooks.SQClass.ModVersion("{VERSION}")).tostring()"#)
		),
		"true",
		"compared as versions"
	);
	assert_eq!(popup_text(&engine), "", "no errors");
	assert_eq!(eval(&engine, "(\"bb_test_queue_ran\" in ::getroottable()).tostring()"), "true");
}

#[test]
fn an_unmet_requirement_shows_modern_hooks_error() {
	let Some(engine) = preloaded("unmet", ">= 99.0.0") else {
		return;
	};
	let text = popup_text(&engine);
	assert!(
		text.contains(&format!("mod_modern_engine (Modern Engine) version {VERSION} is outdated for mod_zz_requires_engine")),
		"Modern Hooks' error: {text:?}"
	);
	assert_eq!(eval(&engine, "::Hooks.Popup.m.ForceQuit.tostring()"), "true", "it quits the game");
}
