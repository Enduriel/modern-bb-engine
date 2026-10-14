//! Atlases load on demand: a live frame skips a sprite whose atlas is
//! still loading, a frame for a screenshot waits for it, and later frames reuse it.
//! Skips when no GPU adapter is available.

use std::collections::HashMap;
use std::sync::Arc;

use bb_assets::{
	BrushFrame, BrushSheet, BrushSprite, FileSource, SheetHeader, SpriteRegistry, name_hash,
};
use bb_render::world::WorldView;
use bb_render::{Atlases, Gpu, RenderTarget, WorldRenderer};
use squirrel_bb::scene::{SceneTile, WorldScene};

struct Files(HashMap<String, Vec<u8>>);

impl FileSource for Files {
	fn read(&self, path: &str) -> Option<Vec<u8>> {
		self.0.get(path).cloned()
	}
}

fn atlas_png() -> Vec<u8> {
	let mut out = Vec::new();
	let mut encoder = png::Encoder::new(&mut out, 2, 2);
	encoder.set_color(png::ColorType::Rgba);
	encoder.set_depth(png::BitDepth::Eight);
	let mut writer = encoder.write_header().unwrap();
	writer.write_image_data(&[255; 16]).unwrap();
	writer.finish().unwrap();
	out
}

fn registry() -> SpriteRegistry {
	let sprite = BrushSprite {
		hash: name_hash(b"dot"),
		name: "dot".into(),
		material: 0,
		width: 2,
		height: 2,
		offset_x: 0,
		offset_y: 0,
		depth_bias: 0,
		frame_time_ms: 100,
		unknown_pause: [0, 0],
		rotation_speed: 0.0,
		flags: 0x403,
		unknown_offset: [0.0, 0.0],
		color: [255; 4],
		unknown_scale: 2.0,
		frames: vec![BrushFrame {
			name: String::new(),
			uv: [0.0, 1.0, 0.0, 1.0],
			rect: [-1.0, 1.0, -1.0, 1.0],
		}],
		trailer: [0; 3],
	};
	let header = SheetHeader {
		width: 2,
		height: 2,
		flags: [0; 12],
		sprite_count: 1,
		reserved: [0; 2],
		max_level: 1000,
	};
	let mut registry = SpriteRegistry::new();
	registry.add_sheet(
		"brushes/test.brush",
		BrushSheet {
			texture: "gfx/test.png".into(),
			header,
			sprites: vec![sprite],
		},
	);
	registry
}

#[test]
fn atlases_load_on_first_use() {
	let gpu = match Gpu::headless() {
		Ok(gpu) => gpu,
		Err(e) => {
			eprintln!("skipping: {e}");
			return;
		}
	};
	let files = Files(HashMap::from([("gfx/test.png".to_string(), atlas_png())]));
	let mut renderer = WorldRenderer::new(&gpu, Arc::new(files), 1);
	let scene = WorldScene {
		tiles: vec![SceneTile {
			col: 0,
			row: 0,
			pos: (0.0, 0.0),
			terrain: 2,
			brush: "dot".into(),
			details: Vec::new(),
		}],
		entities: Vec::new(),
	};
	let registry = registry();
	let view = WorldView {
		center: [0.0, 0.0],
		zoom: 1.0,
		width: 4,
		height: 4,
	};
	let target = RenderTarget::new(&gpu, 4, 4);

	let live = renderer.draw(&gpu, &target, &scene, &registry, &view, Atlases::DrawReady);
	assert_eq!(
		(live.quads, live.waiting),
		(0, 1),
		"the atlas has only started loading"
	);
	let shot = renderer.draw(&gpu, &target, &scene, &registry, &view, Atlases::WaitForAll);
	assert_eq!((shot.quads, shot.waiting), (1, 0));
	let again = renderer.draw(&gpu, &target, &scene, &registry, &view, Atlases::DrawReady);
	assert_eq!((again.quads, again.waiting), (1, 0), "loaded once, reused");

	let image = target.read(&gpu).expect("read back");
	assert_eq!(
		&image.rgba[(5 * 4)..(5 * 4 + 4)],
		&[255, 255, 255, 255],
		"the sprite is drawn"
	);
}
