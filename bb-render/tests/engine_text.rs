//! Engine-drawn text in Simplified Chinese: settlement labels and region names the
//! bitmap fonts can't draw are shaped in the outline fonts, one glyph per character,
//! while vanilla's Windows-1252 text keeps its bitmap font. Checked by code points and
//! glyph pages (and ink on screen), not by eye. Needs the Steam install (skips without).

use std::cell::RefCell;
use std::path::Path;
use std::sync::Arc;

use bb_assets::{BitmapFont, SpriteRegistry};
use bb_engine::files::GameFiles;
use bb_engine::harness::{self, STEAM_DATA_DIR};
use bb_render::fonts::shaper::{GLYPH_PAGES, TextShaper};
use bb_render::fonts::{FontDb, Registration};
use bb_render::labels::LabelFont;
use bb_render::world::{WorldFonts, WorldView, world_quads};
use bb_render::{Atlases, Gpu, RenderTarget, WorldRenderer};
use squirrel_bb::scene::{SceneEntity, SceneLabel, SceneRegionText, WorldScene};

const HAN_FONT: &[u8] = include_bytes!("fixtures/bb_test_han_serif.otf");
const SETTLEMENT: &str = "白石镇";
const REGION: &str = "黑森林";

fn files() -> Option<GameFiles> {
	let archives = harness::data_archives(Path::new(STEAM_DATA_DIR)).ok().filter(|a| !a.is_empty())?;
	GameFiles::open(archives).ok()
}

fn han_font() -> Registration {
	Registration {
		path: "gfx/test/bb_test_han_serif.otf".into(),
		data: HAN_FONT.to_vec(),
		stands_in_for: None,
		scripts: Some(vec!["Han".into()]),
		priority: 0,
	}
}

fn label(text: &str) -> SceneLabel {
	SceneLabel {
		name: "name".into(),
		text: text.as_bytes().into(),
		visible: true,
		color: 0xffff_ffff,
		offset: (0.0, 0.0),
	}
}

/// A settlement named in Chinese, one in English below it, and a Chinese region name.
fn scene() -> WorldScene {
	let entity = |id: i64, y: f64, text: &str| SceneEntity {
		id,
		pos: (-60.0, y),
		shown: true,
		sprites: Vec::new(),
		labels: vec![label(text)],
	};
	WorldScene {
		entities: vec![entity(1, 120.0, SETTLEMENT), entity(2, -40.0, "Lorrested")],
		region_texts: vec![SceneRegionText { text: REGION.as_bytes().into(), scale: 0.5, rotation: 0.0, pos: (0.0, -160.0), id: 3 }],
		..WorldScene::default()
	}
}

fn view() -> WorldView {
	WorldView { center: [0.0, 0.0], zoom: 1.0, width: 640, height: 480 }
}

#[test]
fn chinese_names_are_shaped_one_glyph_a_character_and_latin_keeps_the_bitmap_font() {
	let Some(files) = files() else {
		bb_engine::harness::skip("no install");
		return;
	};
	let bitmap = |name: &str| BitmapFont::parse(&files.read(&format!("gfx/fonts/{name}.fnt")).expect("the bitmap font")).expect("parse");
	let (label_font, region_font) = (bitmap("cinzel_bold_20"), bitmap("cinzel_bold_100"));
	let mut db = FontDb::new(Vec::new(), false);
	db.register(han_font()).expect("the test font");
	let shaper = RefCell::new(TextShaper::new(db));
	let fonts = WorldFonts {
		label: Some(LabelFont { font: &label_font, texture: "gfx/fonts/cinzel_bold_20.png" }),
		region: Some(LabelFont { font: &region_font, texture: "gfx/fonts/cinzel_bold_100.png" }),
		shaper: Some(&shaper),
	};
	let scene = scene();
	let registry = SpriteRegistry::new();
	let quads = world_quads(&scene, &registry, &view(), fonts);
	let key = |q: &bb_render::sprite::Quad| quads.atlases.paths[q.texture].as_str();
	let glyphs = quads.quads.iter().filter(|q| GLYPH_PAGES.contains(&key(q))).count();
	let bitmap_glyphs = quads.quads.iter().filter(|q| key(q) == "gfx/fonts/cinzel_bold_20.png").count();
	assert_eq!(glyphs, 6, "three characters a name, from the glyph pages");
	assert_eq!(bitmap_glyphs, "Lorrested".len(), "the English name keeps the bitmap font");
	// By code point: each glyph stands for one of the name's characters, in order.
	for (bitmap, text) in [("cinzel_bold_20", SETTLEMENT), ("cinzel_bold_100", REGION)] {
		let shaped = shaper.borrow_mut().shape(bitmap, text, None).clone();
		let chars: Vec<&str> = shaped.glyphs.iter().map(|g| &text[g.start..g.end]).collect();
		let expected: Vec<String> = text.chars().map(String::from).collect();
		assert_eq!(chars, expected, "{text} in {bitmap}");
	}
}

#[test]
fn a_registered_font_draws_a_chinese_label_on_screen() {
	let Some(files) = files() else {
		bb_engine::harness::skip("no install");
		return;
	};
	let gpu = match Gpu::headless() {
		Ok(gpu) => gpu,
		Err(e) => {
			bb_engine::harness::skip(&e);
			return;
		}
	};
	let mut renderer = WorldRenderer::new(&gpu, Arc::new(files), 2);
	assert_eq!(renderer.register_font(han_font()).as_deref(), Ok("BB Test Han Serif"));
	let target = RenderTarget::new(&gpu, 640, 480);
	let only = |text: &str| WorldScene {
		entities: vec![SceneEntity { id: 1, pos: (-60.0, 0.0), shown: true, sprites: Vec::new(), labels: vec![label(text)] }],
		..WorldScene::default()
	};
	let registry = SpriteRegistry::new();
	let mut ink = |scene: &WorldScene| {
		renderer.draw(&gpu, &target, scene, &registry, &view(), Atlases::WaitForAll);
		let image = target.read(&gpu).expect("read back");
		image.rgba.chunks(4).filter(|p| p[0] > 200 && p[1] > 200 && p[2] > 200).count()
	};
	let (chinese, plate_only) = (ink(&only(SETTLEMENT)), ink(&only(" ")));
	if let Some(out) = std::env::var_os("BB_TEXT_SHOT") {
		renderer.draw(&gpu, &target, &scene(), &registry, &view(), Atlases::WaitForAll);
		bb_render::save_png(out.to_str().unwrap(), &target.read(&gpu).unwrap()).unwrap();
	}
	assert!(chinese > 100, "the glyphs' white ink: {chinese} pixels");
	assert_eq!(plate_only, 0, "a blank label draws only its dark plate");
}

/// A mod registers its font from its preload script (`ModernEngine.registerFont`, the
/// test mod zip in `fixtures/`), and the label text it's for resolves to that font.
#[test]
fn a_mod_registers_its_font_through_modern_engine() {
	let dir = Path::new(STEAM_DATA_DIR);
	let Some(mut archives) = harness::data_archives(dir).ok().filter(|a| !a.is_empty()) else {
		bb_engine::harness::skip("no install");
		return;
	};
	archives.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mod_han_font_test.zip"));
	let engine = harness::open_mounted(dir, archives, |_| {}).expect("the install and the mod");
	harness::boot_menu(&engine).expect("the menu, the mod's preload run");
	let fonts = engine.font_registrations(0);
	assert_eq!(fonts.len(), 1, "the mod's one font");
	assert_eq!(fonts[0].family, "BB Test Han Serif");
	let mut db = FontDb::new(Vec::new(), false);
	for font in fonts {
		db.register(Registration::of(&font).expect("for labels")).expect("registered");
	}
	let families = |db: &mut FontDb, bitmap: &str| db.runs(bitmap, SETTLEMENT).into_iter().map(|r| r.family).collect::<Vec<_>>();
	assert_eq!(families(&mut db, "cinzel_bold_20"), ["BB Test Han Serif"], "labels");
	assert_eq!(families(&mut db, "cinzel_20"), ["Cinzel"], "not a font it stands in for");
}
