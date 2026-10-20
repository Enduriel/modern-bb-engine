//! Atlases load on demand: a live frame skips a sprite whose atlas is
//! still loading, a frame for a screenshot waits for it, and later frames reuse it.
//! A missing atlas draws the "texture missing" picture. The fog of war covers what the
//! engine's mask keeps fogged. Skips when no GPU adapter is available.

use std::collections::HashMap;
use std::sync::Arc;

use bb_assets::{
	BrushFrame, BrushSheet, BrushSprite, FileSource, SheetHeader, SpriteRegistry, name_hash,
};
use bb_render::world::WorldView;
use bb_render::{Atlases, Discard, Gpu, RenderTarget, WorldRenderer};
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
	renderer.draw_whole_atlases();
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

	// What it holds: the 2×2 atlas, without mips (drawn at its size), besides its own.
	let memory = renderer.memory();
	let atlas = memory.textures.iter().find(|t| t.path == "gfx/test.png").expect("the atlas counted");
	assert_eq!((atlas.width, atlas.height, atlas.bytes, atlas.stand_in), (2, 2, 16, false));
	assert!(memory.own > 0 && memory.total() >= memory.texture_bytes() + memory.own);

	// Drawn smaller than its texels, it gets its mips (the 1×1 level) for that frame.
	let far = WorldView { zoom: 0.25, ..view };
	renderer.draw(&gpu, &target, &scene, &registry, &far, Atlases::DrawReady);
	let atlas_bytes = |r: &WorldRenderer| r.memory().textures.iter().find(|t| t.path == "gfx/test.png").map(|t| t.bytes);
	assert_eq!(atlas_bytes(&renderer), Some(20), "with its 1×1 level");
}

/// A hinted atlas waits on the CPU until a frame draws it; then it's on the GPU until the
/// scene that drew it is discarded: a battle's end keeps what the world map drew, the
/// campaign's end doesn't.
#[test]
fn a_hinted_atlas_goes_to_the_gpu_when_drawn_and_leaves_when_unused() {
	let gpu = match Gpu::headless() {
		Ok(gpu) => gpu,
		Err(e) => {
			eprintln!("skipping: {e}");
			return;
		}
	};
	let files = Files(HashMap::from([("gfx/test.png".to_string(), atlas_png())]));
	let mut renderer = WorldRenderer::new(&gpu, Arc::new(files), 1);
	renderer.draw_whole_atlases();
	let registry = registry();
	let view = WorldView {
		center: [0.0, 0.0],
		zoom: 1.0,
		width: 4,
		height: 4,
	};
	let target = RenderTarget::new(&gpu, 4, 4);
	let empty = WorldScene::default();
	let on_gpu = |r: &WorldRenderer| r.memory().textures.iter().any(|t| t.path == "gfx/test.png");

	renderer.prefetch(&registry, ["gfx/test.png".to_string()]);
	let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
	while renderer.memory().held == 0 {
		assert!(std::time::Instant::now() < deadline, "never decoded");
		renderer.draw(&gpu, &target, &empty, &registry, &view, Atlases::DrawReady);
	}
	assert!(!on_gpu(&renderer), "hinted, not drawn: waiting on the CPU");

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
		..WorldScene::default()
	};
	let stats = renderer.draw(&gpu, &target, &scene, &registry, &view, Atlases::DrawReady);
	assert_eq!((stats.quads, stats.waiting), (1, 0), "drawn in the same frame it was needed");
	assert!(on_gpu(&renderer) && renderer.memory().held == 0);

	renderer.begin_frame();
	renderer.discard(Discard::Battle);
	assert!(on_gpu(&renderer), "the world map's, after a battle");
	renderer.discard(Discard::Campaign);
	assert!(!on_gpu(&renderer), "the campaign left: dropped");
}

/// A missing atlas draws the game's "texture missing" picture (`error.png`) in its
/// place, as the original does; without that picture, its sprites aren't drawn.
#[test]
fn a_missing_atlas_draws_the_texture_missing_picture() {
	let gpu = match Gpu::headless() {
		Ok(gpu) => gpu,
		Err(e) => {
			eprintln!("skipping: {e}");
			return;
		}
	};
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
		..WorldScene::default()
	};
	let registry = registry();
	let view = WorldView {
		center: [0.0, 0.0],
		zoom: 1.0,
		width: 4,
		height: 4,
	};
	let red = [255, 0, 0, 255];
	for (files, drawn) in [(HashMap::from([("error.png".to_string(), solid_png(red))]), 1), (HashMap::new(), 0)] {
		let mut renderer = WorldRenderer::new(&gpu, Arc::new(Files(files)), 1);
		let target = RenderTarget::new(&gpu, 4, 4);
		let stats = renderer.draw(&gpu, &target, &scene, &registry, &view, Atlases::WaitForAll);
		assert_eq!((stats.quads, stats.failed), (drawn, 1), "gfx/test.png is missing");
		let memory = renderer.memory();
		let stand_in = memory.textures.iter().find(|t| t.path == "gfx/test.png");
		assert_eq!(stand_in.map(|t| t.stand_in), (drawn == 1).then_some(true), "counted as a stand-in");
		// The water's picture is missing here too.
		let failed = if drawn == 1 { 0 } else { 2 };
		assert_eq!(memory.failed, failed, "or as failed, with nothing in its place");
		if drawn == 1 {
			let image = target.read(&gpu).expect("read back");
			assert_eq!(&image.rgba[(5 * 4)..(5 * 4 + 4)], &red, "drawn with the picture");
		}
	}
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
				brush: "".into(),
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

/// A sprite drawn from one of our pages of frames is drawn exactly as from its atlas,
/// at its size and zoomed out (from a page with mips); a hint at its atlas doesn't decode
/// the atlas whole; its frame waits one frame for the workers, and goes with the campaign.
#[test]
fn a_frame_from_a_page_draws_as_from_its_atlas() {
	let gpu = match Gpu::headless() {
		Ok(gpu) => gpu,
		Err(e) => {
			eprintln!("skipping: {e}");
			return;
		}
	};
	// Four texels of different colours, so filtering and mips show.
	let mut out = Vec::new();
	let mut encoder = png::Encoder::new(&mut out, 2, 2);
	encoder.set_color(png::ColorType::Rgba);
	encoder.set_depth(png::BitDepth::Eight);
	let mut writer = encoder.write_header().unwrap();
	writer.write_image_data(&[[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255], [255, 255, 0, 128]].concat()).unwrap();
	writer.finish().unwrap();
	let files = || Arc::new(Files(HashMap::from([("gfx/test.png".to_string(), out.clone())])));
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
		..WorldScene::default()
	};
	let registry = registry();
	for zoom in [1.0, 4.0, 0.5] {
		let view = WorldView {
			center: [0.0, 0.0],
			zoom,
			width: 8,
			height: 8,
		};
		let draw = |renderer: &mut WorldRenderer| {
			let target = RenderTarget::new(&gpu, 8, 8);
			let stats = renderer.draw(&gpu, &target, &scene, &registry, &view, Atlases::WaitForAll);
			assert_eq!((stats.quads, stats.waiting), (1, 0));
			target.read(&gpu).expect("read back")
		};
		let mut whole = WorldRenderer::new(&gpu, files(), 1);
		whole.draw_whole_atlases();
		let mut paged = WorldRenderer::new(&gpu, files(), 1);
		assert!(draw(&mut whole) == draw(&mut paged), "the same pixels at zoom {zoom}");
		let memory = paged.memory();
		assert_eq!((memory.frames, memory.pages), (1, 1));
		assert!(!memory.textures.iter().any(|t| t.path == "gfx/test.png"), "the atlas itself isn't on the GPU");
	}

	// Live, the frame comes from the workers (within the frame's wait budget, or a frame
	// or more later), then draws; leaving the campaign drops it.
	let mut renderer = WorldRenderer::new(&gpu, files(), 1);
	let view = WorldView {
		center: [0.0, 0.0],
		zoom: 1.0,
		width: 8,
		height: 8,
	};
	let target = RenderTarget::new(&gpu, 8, 8);
	// A hint readies the atlas's sheet; the atlas isn't decoded whole for the CPU to hold.
	renderer.prefetch(&registry, ["gfx/test.png".to_string()]);
	assert_eq!(renderer.memory().held, 0, "nothing warmed whole");
	let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
	loop {
		renderer.begin_frame();
		let stats = renderer.draw(&gpu, &target, &scene, &registry, &view, Atlases::DrawReady);
		if stats.quads == 1 {
			break;
		}
		assert!(std::time::Instant::now() < deadline, "never arrived");
		std::thread::sleep(std::time::Duration::from_millis(1));
	}
	renderer.begin_frame();
	renderer.discard(Discard::Campaign);
	assert_eq!(renderer.memory().frames, 0, "the campaign left: its frames go");
}

/// An animated sprite's frames load together: the first draw of its first frame brings
/// the rest, so it never waits on its next frame.
#[test]
fn an_animations_frames_load_together() {
	let gpu = match Gpu::headless() {
		Ok(gpu) => gpu,
		Err(e) => {
			eprintln!("skipping: {e}");
			return;
		}
	};
	let mut registry = registry();
	let frame = |u0: f32, u1: f32| BrushFrame {
		name: String::new(),
		uv: [u0, u1, 0.0, 1.0],
		rect: [-1.0, 1.0, -1.0, 1.0],
	};
	let anim = BrushSprite {
		hash: name_hash(b"anim"),
		name: "anim".into(),
		material: 0,
		width: 1,
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
		frames: vec![frame(0.0, 0.5), frame(0.5, 1.0)],
		trailer: [0; 3],
	};
	registry.add_sheet(
		"brushes/anim.brush",
		BrushSheet {
			texture: "gfx/test.png".into(),
			header: SheetHeader {
				width: 2,
				height: 2,
				flags: [0; 12],
				sprite_count: 1,
				reserved: [0; 2],
				max_level: 1000,
			},
			sprites: vec![anim],
		},
	);
	let files = Files(HashMap::from([("gfx/test.png".to_string(), atlas_png())]));
	let mut renderer = WorldRenderer::new(&gpu, Arc::new(files), 1);
	let scene = WorldScene {
		tiles: vec![SceneTile {
			col: 0,
			row: 0,
			pos: (0.0, 0.0),
			terrain: 2,
			discovered: true,
			brush: "anim".into(),
			flipped: false,
			details: Vec::new(),
		}],
		..WorldScene::default()
	};
	let view = WorldView {
		center: [0.0, 0.0],
		zoom: 1.0,
		width: 8,
		height: 8,
	};
	let target = RenderTarget::new(&gpu, 8, 8);
	let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
	loop {
		renderer.begin_frame();
		if renderer.draw(&gpu, &target, &scene, &registry, &view, Atlases::DrawReady).quads == 1 {
			break;
		}
		assert!(std::time::Instant::now() < deadline, "never arrived");
		std::thread::sleep(std::time::Duration::from_millis(1));
	}
	assert_eq!(renderer.memory().frames, 2, "its second frame came with the first");
}

/// The cursor draws from a page too, unscaled at its position, as from its atlas.
#[test]
fn the_cursor_draws_from_a_page_as_from_its_atlas() {
	let gpu = match Gpu::headless() {
		Ok(gpu) => gpu,
		Err(e) => {
			eprintln!("skipping: {e}");
			return;
		}
	};
	let registry = registry();
	let draw = |renderer: &mut WorldRenderer| {
		let target = RenderTarget::new(&gpu, 8, 8);
		let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
		loop {
			renderer.begin_frame();
			let stats = renderer.draw_cursor(&gpu, &target, &registry, "dot", 0.0, [0.0, 0.0]);
			if stats.quads == 1 {
				break;
			}
			assert!(std::time::Instant::now() < deadline, "never drawn: {stats:?}");
			std::thread::sleep(std::time::Duration::from_millis(1));
		}
		target.read(&gpu).expect("read back")
	};
	let files = || Arc::new(Files(HashMap::from([("gfx/test.png".to_string(), atlas_png())])));
	let mut whole = WorldRenderer::new(&gpu, files(), 1);
	whole.draw_whole_atlases();
	let mut paged = WorldRenderer::new(&gpu, files(), 1);
	let (a, b) = (draw(&mut whole), draw(&mut paged));
	assert!(a.rgba.iter().any(|&v| v != 0), "the cursor shows");
	assert!(a == b, "the same pixels");
}

/// A map kept from frame to frame (`draw_kept`) is built once per revision, and a detail
/// whose look changed is drawn anew, a hidden one shown again in its place.
#[test]
fn a_kept_map_redraws_only_what_changed() {
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
		width: 8,
		height: 4,
	};
	let target = RenderTarget::new(&gpu, 8, 4);
	let detail = |id: i64, x: f32, hidden: bool| SceneDetail {
		id,
		hidden,
		brush: "dot".into(),
		pos: (x, 0.0),
		..SceneDetail::default()
	};
	let tiles = |details: Vec<SceneDetail>| {
		vec![SceneTile {
			col: 0,
			row: 0,
			pos: (0.0, 0.0),
			terrain: 2,
			discovered: true,
			brush: "".into(),
			flipped: false,
			details,
		}]
	};
	let scene = WorldScene {
		fog_of_war: false,
		..WorldScene::default()
	};
	let built = std::cell::Cell::new(0);
	let mut draw = |revision: u64, map: Vec<SceneDetail>, changes: &[SceneDetail]| {
		let whole = || {
			built.set(built.get() + 1);
			tiles(map.clone())
		};
		let map = bb_render::WorldMap {
			revision,
			tiles: &whole,
			changes,
		};
		renderer.draw_kept(&gpu, &target, &scene, &map, &registry, &view, Atlases::WaitForAll);
		let image = target.read(&gpu).expect("read back");
		// Left of the centre, and right of it (a row in the middle).
		let px = |x: usize| <[u8; 4]>::try_from(&image.rgba[(8 + x) * 4..(8 + x) * 4 + 4]).unwrap();
		(px(1), px(5))
	};
	let map = vec![detail(1, -2.0, false), detail(2, 2.0, true)];
	assert_eq!(draw(1, map.clone(), &[]), (RED, [0, 0, 0, 255]), "the hidden one isn't drawn");
	assert_eq!(draw(1, map.clone(), &[]).0, RED);
	assert_eq!(built.get(), 1, "built once");

	let dark = SceneDetail { tint: 0, ..detail(1, -2.0, false) };
	let (left, right) = draw(1, map.clone(), &[dark, detail(2, 2.0, false)]);
	assert_eq!((left[0], right), (0, RED), "darkened, and shown again");
	assert_eq!(built.get(), 1, "only the changed quads were written");

	// A new revision builds it again: one detail left.
	assert_eq!(draw(2, vec![detail(2, 2.0, false)], &[]), ([0, 0, 0, 255], RED));
	assert_eq!(built.get(), 2);
}
