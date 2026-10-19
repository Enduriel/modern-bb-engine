//! `ModernEngine`: the root table mods detect this engine by, and its `registerFont`
//! (`docs/modding.md`).

use std::collections::HashMap;

use squirrel::SqTable;
use squirrel_bb::{Engine, LoadedScript, ScriptSource};

const FONT: &[u8] = include_bytes!("../../bb-render/tests/fixtures/bb_test_han_serif.otf");

/// No scripts; a mod's font and a file that isn't one.
struct Files(HashMap<&'static str, Vec<u8>>);

impl ScriptSource for Files {
	fn load(&self, _path: &str) -> Option<LoadedScript> {
		None
	}
	fn list_prefix(&self, _prefix: &str) -> Option<Vec<String>> {
		None
	}
	fn read_file(&self, path: &str) -> Option<Vec<u8>> {
		self.0.get(path).cloned()
	}
}

fn engine() -> Engine<Files> {
	let files = Files(HashMap::from([
		("gfx/mod_cn/fonts/han.otf", FONT.to_vec()),
		("gfx/mod_cn/readme.txt", b"not a font".to_vec()),
	]));
	let mut engine = Engine::new(files);
	engine.vm_mut().set_print_handler(|_| {});
	engine
}

/// The string the script left in the root table's `key`.
fn root_string(engine: &Engine<Files>, key: &str) -> Option<String> {
	SqTable::root(engine.vm()).get::<String>(key)
}

#[test]
fn mods_detect_the_engine_and_register_fonts_in_load_order() {
	let engine = engine();
	engine
		.run_script(
			r#"
			::found <- ("ModernEngine" in ::getroottable()) ? "yes " + ::ModernEngine.Version : "no";
			::family <- ::ModernEngine.registerFont("gfx/mod_cn/fonts/han.otf");
			::ModernEngine.registerFont("gfx/mod_cn/fonts/han.otf", { For = ["cinzel_bold_20", "cinzel_bold_100"], Scripts = "Han", Priority = 2 });
			"#,
			"mod",
		)
		.expect("registered");
	assert_eq!(root_string(&engine, "found").as_deref(), Some("yes 1"));
	assert_eq!(root_string(&engine, "family").as_deref(), Some("BB Test Han Serif"));
	let fonts = engine.take_font_registrations();
	assert_eq!(fonts.len(), 2);
	assert_eq!((fonts[0].stands_in_for.clone(), fonts[0].scripts.clone(), fonts[0].priority), (None, None, 0), "the defaults");
	assert_eq!(fonts[1].stands_in_for.as_deref(), Some(&["cinzel_bold_20".to_string(), "cinzel_bold_100".to_string()][..]));
	assert_eq!(fonts[1].scripts.as_deref(), Some(&["Han".to_string()][..]));
	assert_eq!(fonts[1].priority, 2);
	assert_eq!(fonts[1].data, FONT);
	assert!(engine.take_font_registrations().is_empty(), "taken once");
}

#[test]
fn mistakes_throw_with_a_reason() {
	let engine = engine();
	let error = |call: &str| -> String {
		engine
			.run_script(&format!(r#"::err <- ""; try {{ {call}; }} catch (e) {{ ::err = e; }}"#), "mod")
			.expect("ran");
		root_string(&engine, "err").unwrap_or_default()
	};
	assert!(error(r#"::ModernEngine.registerFont("gfx/mod_cn/fonts/none.otf")"#).contains("no such file"));
	assert!(error(r#"::ModernEngine.registerFont("gfx/mod_cn/readme.txt")"#).contains("not a TrueType"));
	assert!(error(r#"::ModernEngine.registerFont("gfx/mod_cn/fonts/han.otf", { For = "cinzel_30" })"#).contains("cinzel_30"));
	assert!(error(r#"::ModernEngine.registerFont("gfx/mod_cn/fonts/han.otf", { Scripts = ["Han", "Klingon"] })"#).contains("Klingon"));
	assert!(error(r#"::ModernEngine.registerFont("gfx/mod_cn/fonts/han.otf", { Priority = "high" })"#).contains("Priority"));
	assert!(engine.take_font_registrations().is_empty(), "nothing kept");
}
