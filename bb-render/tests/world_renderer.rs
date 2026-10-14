//! Atlases load on demand: a live frame skips a sprite whose atlas is
//! still loading, a frame for a screenshot waits for it, and later frames reuse it.
//! The fog of war covers what the engine's mask keeps fogged. Skips when no GPU
//! adapter is available.

use std::collections::HashMap;
use std::sync::Arc;

use bb_assets::{
	BrushFrame, BrushSheet, BrushSprite, FileSource, SheetHeader, SpriteRegistry, name_hash,
};
use bb_render::world::WorldView;
use bb_render::{Atlases, Gpu, RenderTarget, WorldRenderer};
use squirrel_bb::scene::{SceneDetail, SceneTile, WorldScene};
use squirrel_bb::{FOG_MASK_SIZE, FogPatch};

struct Files(HashMap<String, Vec<u8>>);

impl FileSource for Files {
	fn read(&self, path: &str) -> Option<Vec<u8>> {
		self.0.get(path).cloned()
	}
}

/// A 2×2 PNG of one colour.
fn solid_png(rgba: [u8; 4]) -> Vec<u8> {
	columns_png(rgba, rgba)
}

/// A 2×2 PNG, its left column `left` and its right `right`.
fn columns_png(left: [u8; 4], right: [u8; 4]) -> Vec<u8> {
	let mut out = Vec::new();
	let mut encoder = png::Encoder::new(&mut out, 2, 2);
	encoder.set_color(png::ColorType::Rgba);
	encoder.set_depth(png::BitDepth::Eight);
	let mut writer = encoder.write_header().unwrap();
	writer.write_image_data(&[left, right, left, right].concat()).unwrap();
	writer.finish().unwrap();
	out
}

fn atlas_png() -> Vec<u8> {
	solid_png([255; 4])
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
			discovered: true,
			brush: "dot".into(),
			flipped: false,
			details: Vec::new(),
		}],
		entities: Vec::new(),
		extent: (0.0, 0.0),
		fog_of_war: false,
		..WorldScene::default()
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

#[test]
fn flipped_tiles_mirror_their_picture_in_place() {
	let gpu = match Gpu::headless() {
		Ok(gpu) => gpu,
		Err(e) => {
			eprintln!("skipping: {e}");
			return;
		}
	};
	const RED: [u8; 4] = [255, 0, 0, 255];
	const BLUE: [u8; 4] = [0, 0, 255, 255];
	let files = Files(HashMap::from([("gfx/test.png".to_string(), columns_png(RED, BLUE))]));
	let mut renderer = WorldRenderer::new(&gpu, Arc::new(files), 1);
	let registry = registry();
	let view = WorldView {
		center: [0.0, 0.0],
		zoom: 1.0,
		width: 4,
		height: 4,
	};
	let target = RenderTarget::new(&gpu, 4, 4);
	let mut left_pixel = |flipped: bool| {
		let scene = WorldScene {
			tiles: vec![SceneTile {
				col: 0,
				row: 0,
				pos: (0.0, 0.0),
				terrain: 2,
				discovered: true,
				brush: "dot".into(),
				flipped,
				details: Vec::new(),
			}],
			fog_of_war: false,
			..WorldScene::default()
		};
		renderer.draw(&gpu, &target, &scene, &registry, &view, Atlases::WaitForAll);
		let image = target.read(&gpu).expect("read back");
		// The 2×2 sprite covers pixels 1 and 2 of row 1.
		let at = (4 + 1) * 4;
		<[u8; 4]>::try_from(&image.rgba[at..at + 4]).unwrap()
	};
	assert_eq!(left_pixel(false), RED);
	assert_eq!(left_pixel(true), BLUE, "the right column shows on the left");
}

#[test]
fn details_draw_with_their_colour_and_saturation() {
	let gpu = match Gpu::headless() {
		Ok(gpu) => gpu,
		Err(e) => {
			eprintln!("skipping: {e}");
			return;
		}
	};
	const RED: [u8; 4] = [255, 0, 0, 255];
	let files = Files(HashMap::from([("gfx/test.png".to_string(), solid_png(RED))]));
	let mut renderer = WorldRenderer::new(&gpu, Arc::new(files), 1);
	let registry = registry();
	let view = WorldView {
		center: [0.0, 0.0],
		zoom: 1.0,
		width: 4,
		height: 4,
	};
	let target = RenderTarget::new(&gpu, 4, 4);
	let mut pixel = |tint: u32, saturation: f64| {
		let mut detail = SceneDetail::default();
		detail.brush = "dot".into();
		detail.tint = tint;
		detail.saturation = saturation;
		let scene = WorldScene {
			tiles: vec![SceneTile {
				col: 0,
				row: 0,
				pos: (0.0, 0.0),
				terrain: 2,
				discovered: true,
				brush: String::new(),
				flipped: false,
				details: vec![detail],
			}],
			fog_of_war: false,
			..WorldScene::default()
		};
		renderer.draw(&gpu, &target, &scene, &registry, &view, Atlases::WaitForAll);
		let image = target.read(&gpu).expect("read back");
		let at = (4 + 1) * 4;
		<[u8; 4]>::try_from(&image.rgba[at..at + 4]).unwrap()
	};
	assert_eq!(pixel(0x00ff_ffff, 1.0), RED);
	let dark = pixel(0x0080_8080, 1.0);
	assert!(dark[0].abs_diff(128) <= 1 && dark[1] == 0, "darkened (setBrightness): {dark:?}");
	let grey = pixel(0x00ff_ffff, 0.0);
	assert!(grey[0] == grey[1] && grey[1] == grey[2] && grey[0] > 0, "desaturated: {grey:?}");
}

#[test]
fn fog_covers_what_the_mask_keeps() {
	let gpu = match Gpu::headless() {
		Ok(gpu) => gpu,
		Err(e) => {
			eprintln!("skipping: {e}");
			return;
		}
	};
	let files = Files(HashMap::from([(
		"gfx/fow.png".to_string(),
		solid_png([100, 100, 100, 255]),
	)]));
	let mut renderer = WorldRenderer::new(&gpu, Arc::new(files), 1);
	// A 4×4-unit map, fogged on its left half and uncovered on its right.
	let size = FOG_MASK_SIZE as u32;
	let alpha = (0..size * size)
		.map(|i| if i % size < size / 2 { 255 } else { 0 })
		.collect();
	let scene = WorldScene {
		tiles: Vec::new(),
		entities: Vec::new(),
		extent: (4.0, 4.0),
		fog_of_war: true,
		..WorldScene::default()
	};
	let view = WorldView {
		center: [2.0, 2.0],
		zoom: 1.0,
		width: 4,
		height: 4,
	};
	let target = RenderTarget::new(&gpu, 4, 4);
	let registry = SpriteRegistry::new();
	let pixel = |image: &bb_assets::Image, x: usize, y: usize| {
		let at = (y * 4 + x) * 4;
		image.rgba[at..at + 4].to_vec()
	};

	renderer.draw(&gpu, &target, &scene, &registry, &view, Atlases::WaitForAll);
	let image = target.read(&gpu).expect("read back");
	assert_eq!(pixel(&image, 0, 1), [0, 0, 0, 255], "no mask yet, no fog");

	renderer.update_fog(
		&gpu,
		&FogPatch {
			mask: size,
			x: 0,
			y: 0,
			width: size,
			height: size,
			alpha,
		},
	);
	renderer.draw(&gpu, &target, &scene, &registry, &view, Atlases::WaitForAll);
	let image = target.read(&gpu).expect("read back");
	// Three samples at a third each: 99 of the texture's 100.
	assert_eq!(pixel(&image, 0, 1), [99, 99, 99, 255], "fogged");
	assert_eq!(pixel(&image, 3, 1), [0, 0, 0, 255], "uncovered");

	// A larger map's mask (twice vanilla's width gets 4096²) replaces the texture,
	// here fogged on the right and uncovered on the left.
	let larger = 2 * size;
	renderer.update_fog(
		&gpu,
		&FogPatch {
			mask: larger,
			x: 0,
			y: 0,
			width: larger,
			height: larger,
			alpha: (0..larger * larger).map(|i| if i % larger < larger / 2 { 0 } else { 255 }).collect(),
		},
	);
	renderer.draw(&gpu, &target, &scene, &registry, &view, Atlases::WaitForAll);
	let image = target.read(&gpu).expect("read back");
	assert_eq!(pixel(&image, 0, 1), [0, 0, 0, 255], "the larger mask: uncovered");
	assert_eq!(pixel(&image, 3, 1), [99, 99, 99, 255], "the larger mask: fogged");

	let off = WorldScene {
		fog_of_war: false,
		..scene
	};
	renderer.draw(&gpu, &target, &off, &registry, &view, Atlases::WaitForAll);
	let image = target.read(&gpu).expect("read back");
	assert_eq!(pixel(&image, 0, 1), [0, 0, 0, 255], "fog of war off");
}
