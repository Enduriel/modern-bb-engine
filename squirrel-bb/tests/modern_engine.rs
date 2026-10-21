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
	assert_eq!(root_string(&engine, "found").as_deref(), Some("yes 3"));
	assert_eq!(root_string(&engine, "family").as_deref(), Some("BB Test Han Serif"));
	let fonts = engine.font_registrations(0);
	assert_eq!(fonts.len(), 2);
	assert_eq!((fonts[0].stands_in_for.clone(), fonts[0].scripts.clone(), fonts[0].priority), (None, None, 0), "the defaults");
	assert_eq!(fonts[1].stands_in_for.as_deref(), Some(&["cinzel_bold_20".to_string(), "cinzel_bold_100".to_string()][..]));
	assert_eq!(fonts[1].scripts.as_deref(), Some(&["Han".to_string()][..]));
	assert_eq!(fonts[1].priority, 2);
	assert_eq!(fonts[1].data, FONT);
	assert_eq!(fonts[0].ui_families(), squirrel_bb::modern_engine::UI_FAMILIES, "the page's every family");
	assert_eq!(fonts[1].ui_families(), ["Cinzel-Bold"], "the bitmap fonts' typeface on the page");
	let has = |range: &str, c: char| {
		range.split(", ").any(|r| {
			let (start, end) = r.trim_start_matches("U+").split_once('-').unwrap_or((r.trim_start_matches("U+"), r.trim_start_matches("U+")));
			(u32::from_str_radix(start, 16).unwrap()..=u32::from_str_radix(end, 16).unwrap()).contains(&u32::from(c))
		})
	};
	let all = fonts[0].ui_unicode_range();
	assert!(has(&all, '中') && has(&all, 'я') && !has(&all, 'A') && !has(&all, 'é'), "never vanilla's own characters");
	let han = fonts[1].ui_unicode_range();
	assert!(has(&han, '中') && !has(&han, 'あ') && !has(&han, 'A'), "Han, not Hiragana: {han}");
	assert_eq!(engine.font_registrations(1).len(), 1, "from the second on");
	assert!(engine.font_registrations(2).is_empty(), "none past the last");
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
	assert!(engine.font_registrations(0).is_empty(), "nothing kept");
	engine
		.run_script(r#"::ModernEngine.registerFont("gfx/mod_cn/fonts/han.otf", { For = ["FreeUniversal-Regular", "cinzel_20"] });"#, "mod")
		.expect("a UI family is a font it can stand in for");
	assert_eq!(engine.font_registrations(0)[0].ui_families(), ["FreeUniversal-Regular", "Cinzel-Regular"]);
}
