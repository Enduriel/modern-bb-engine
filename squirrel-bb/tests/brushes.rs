//! `doesBrushExist` against a registry the host fills, with the argument handling
//! probed in the original (2026-10-13): known names are found, case and whitespace
//! matter, and `""`, `null` or a number answer `false` without an error.

use bb_assets::{BrushFrame, BrushSheet, BrushSprite, SheetHeader, SpriteRegistry, name_hash};
use squirrel::{SqArray, SqTable};
use squirrel_bb::{Engine, LoadedScript, ScriptSource};

struct NoScripts;
impl ScriptSource for NoScripts {
	fn load(&self, _path: &str) -> Option<LoadedScript> {
		None
	}
	fn list_prefix(&self, _prefix: &str) -> Option<Vec<String>> {
		None
	}
}

fn sprite(name: &str) -> BrushSprite {
	BrushSprite {
		hash: name_hash(name.as_bytes()),
		name: name.into(),
		material: 0,
		width: 1,
		height: 1,
		offset_x: 0,
		offset_y: 0,
		depth_bias: 0,
		frame_time_ms: 100,
		unknown_pause: [0, 0],
		rotation_speed: 0.0,
		flags: 0x403,
		pivot: [0.0, 0.0],
		color: [0; 4],
		unknown_scale: 2.0,
		frames: vec![BrushFrame { name: String::new(), uv: [0.0; 4], rect: [0.0; 4] }],
		trailer: [0; 3],
	}
}

fn registry(names: &[&str]) -> SpriteRegistry {
	let sheet = BrushSheet {
		texture: "gfx/test.png".into(),
		header: SheetHeader {
			width: 1,
			height: 1,
			flags: [0; 12],
			sprite_count: names.len() as u32,
			reserved: [0; 2],
			max_level: 1000,
		},
		sprites: names.iter().map(|n| sprite(n)).collect(),
	};
	let mut registry = SpriteRegistry::new();
	registry.add_sheet("brushes/test.brush", sheet);
	registry
}

fn answers(engine: &Engine<NoScripts>, calls: &str) -> Vec<bool> {
	engine.run_script(&format!("::__answers <- [{calls}];"), "brushes").expect("run");
	let array = SqTable::root(engine.vm()).get::<SqArray>("__answers").expect("answers");
	(0..array.len()).map(|i| array.get::<bool>(i).expect("bool")).collect()
}

#[test]
fn nothing_exists_before_the_host_loads_brushes() {
	let engine = Engine::new(NoScripts);
	assert_eq!(answers(&engine, r#"doesBrushExist("world_birds_01")"#), [false]);
}

#[test]
fn answers_like_the_original() {
	let engine = Engine::new(NoScripts);
	engine.sprites().set(registry(&["world_birds_01", "bust_head_dead_bloodpool"]));
	let calls = r#"
		doesBrushExist("bust_head_dead_bloodpool"),
		doesBrushExist("world_birds_01"),
		doesBrushExist(""),
		doesBrushExist("no_such_brush"),
		doesBrushExist("WORLD_BIRDS_01"),
		doesBrushExist("world_birds_01 "),
		doesBrushExist(null),
		doesBrushExist(5)
	"#;
	assert_eq!(answers(&engine, calls), [true, true, false, false, false, false, false, false]);
}
