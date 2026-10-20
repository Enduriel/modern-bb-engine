//! The world map's moving layer kept on the GPU draws what the
//! per-frame build draws, pixel for pixel, frame after frame as items move, change,
//! come and go. Skips when no GPU adapter is available.

use std::collections::HashMap;
use std::sync::Arc;

use bb_assets::{BrushFrame, BrushSheet, BrushSprite, FileSource, SheetHeader, SpriteRegistry, name_hash};
use bb_render::world::WorldView;
use bb_render::{Atlases, Gpu, RenderTarget, WorldMap, WorldRenderer};
use squirrel_bb::scene::{SceneBird, SceneCloud, SceneEntity, SceneFootprint, SceneParticle, SceneSprite, SceneWave, WorldScene};

struct Files(HashMap<String, Vec<u8>>);

impl FileSource for Files {
	fn read(&self, path: &str) -> Option<Vec<u8>> {
		self.0.get(path).cloned()
	}
}

/// A 16×16 PNG whose texels all differ (so a texture coordinate off shows).
fn gradient_png(seed: u8) -> Vec<u8> {
	let mut out = Vec::new();
	let mut encoder = png::Encoder::new(&mut out, 16, 16);
	encoder.set_color(png::ColorType::Rgba);
	encoder.set_depth(png::BitDepth::Eight);
	let mut writer = encoder.write_header().unwrap();
	let texels: Vec<u8> = (0..16u8)
		.flat_map(|y| (0..16u8).flat_map(move |x| [x * 16, y * 16, seed.wrapping_add(x ^ y), 128 + x * 4]))
		.collect();
	writer.write_image_data(&texels).unwrap();
	writer.finish().unwrap();
	out
}

fn sprite(name: &str, rect: [f32; 4], uv: [f32; 4], frames: usize, depth_bias: i8) -> BrushSprite {
	BrushSprite {
		hash: name_hash(name.as_bytes()),
		name: name.into(),
		material: 0,
		width: 16,
		height: 16,
		offset_x: 1,
		offset_y: -2,
		depth_bias,
		frame_time_ms: 100,
		unknown_pause: [0, 0],
		rotation_speed: 0.0,
		flags: 0x403,
		unknown_offset: [0.0, 0.0],
		color: [255; 4],
		unknown_scale: 2.0,
		frames: (0..frames)
			.map(|i| BrushFrame {
				name: String::new(),
				uv: [uv[0], uv[1], uv[2] + i as f32 * 0.125, uv[3] + i as f32 * 0.125],
				rect,
			})
			.collect(),
		trailer: [0; 3],
	}
}

fn registry() -> SpriteRegistry {
	let header = SheetHeader {
		width: 16,
		height: 16,
		flags: [0; 12],
		sprite_count: 0,
		reserved: [0; 2],
		max_level: 1000,
	};
	let mut registry = SpriteRegistry::new();
	registry.add_sheet(
		"brushes/a.brush",
		BrushSheet {
			texture: "gfx/a.png".into(),
			header: SheetHeader { sprite_count: 3, ..header.clone() },
			sprites: vec![
				sprite("body", [-7.0, 6.0, -9.0, 5.0], [0.0, 0.5, 0.0, 0.375], 1, 0),
				sprite("banner", [-3.0, 4.0, -2.0, 8.0], [0.5, 1.0, 0.0, 0.25], 3, 2),
				sprite("world_clouds_01", [-20.0, 18.0, -12.0, 11.0], [0.0, 1.0, 0.5, 1.0], 1, 0),
			],
		},
	);
	registry.add_sheet(
		"brushes/b.brush",
		BrushSheet {
			texture: "gfx/b.png".into(),
			header: SheetHeader { sprite_count: 2, ..header },
			sprites: vec![
				sprite("world_birds_02", [-4.0, 4.0, -3.0, 3.0], [0.0, 0.25, 0.0, 0.125], 4, 0),
				sprite("dust", [-2.0, 3.0, -2.0, 2.0], [0.5, 0.75, 0.5, 0.75], 1, -1),
			],
		},
	);
	registry
}

fn entity(id: i64, pos: (f64, f64), sprites: Vec<SceneSprite>) -> SceneEntity {
	SceneEntity {
		id,
		pos,
		shown: true,
		sprites,
		labels: Vec::new(),
	}
}

fn look(name: &str, brush: &str, color: u32) -> SceneSprite {
	SceneSprite {
		name: name.into(),
		brush: brush.into(),
		color,
		scale: 1.0,
		visible: true,
		offset: (0.0, 0.0),
		flipped: false,
		rotation: 0.0,
		ignore_ambient: false,
		scaling: true,
		colorization: true,
		frame: 0,
	}
}

/// A scene with every kind of item, at step `t`.
fn scene(t: usize) -> WorldScene {
	let mut entities = vec![
		entity(1, (-20.0, 5.0), vec![look("body", "body", 0xffff_ffff), SceneSprite { frame: (t % 3) as u32, offset: (3.0, 4.0), ..look("banner", "banner", 0xc0ff_8040) }]),
		entity(
			2,
			(10.0 + t as f64 * 1.5, -6.0),
			vec![SceneSprite { flipped: true, rotation: 30.0 * t as f32, scale: 1.5, ..look("body", "body", 0x80ff_ffff) }],
		),
		entity(3, (25.0, 12.0), vec![SceneSprite { scaling: false, ..look("banner", "banner", 0xffff_ffff) }]),
	];
	// One comes and goes.
	if t % 2 == 0 {
		entities.insert(1, entity(4, (0.0, 0.0), vec![look("body", "body", 0xff40_ff40)]));
	}
	WorldScene {
		entities,
		extent: (0.0, 0.0),
		fog_of_war: false,
		clouds: vec![SceneCloud {
			brush: "world_clouds_01",
			pos: (5.0 - t as f32, 20.0),
			scale: 1.1,
			alpha: 0.8,
			color: 0xffff_ffff,
			velocity: 40.0,
			key: -9500,
		}],
		birds: vec![SceneBird {
			brush: "world_birds_02",
			pos: (-10.0, -15.0 + t as f32),
			velocity: 50.0,
			key: -4500,
			frame: (t % 4) as u32,
		}],
		footprints: vec![SceneFootprint {
			brush: "dust".into(),
			pos: (-25.0, -20.0),
			scale: 1.0,
			alpha: 255.0 - 20.0 * t as f32,
		}],
		waves: (0..t % 3).map(|i| SceneWave { brush: "dust".into(), pos: (30.0, -20.0 + 6.0 * i as f32), alpha: 200.0 }).collect(),
		particles: (0..(t + 1) % 4)
			.map(|i| SceneParticle {
				brush: "dust".into(),
				pos: (-30.0 + 4.0 * i as f32, 25.0 - t as f32),
				color: 0xa0ff_c080,
				saturation: 1.0,
				scale: 0.5 + 0.25 * i as f32,
				rotation: 15.0 * i as f32,
			})
			.collect(),
		..WorldScene::default()
	}
}

#[test]
fn the_kept_moving_layer_draws_what_the_frame_built_draws() {
	let gpu = match Gpu::headless() {
		Ok(gpu) => gpu,
		Err(e) => {
			eprintln!("skipping: {e}");
			return;
		}
	};
	let files = Files(HashMap::from([
		("gfx/a.png".to_string(), gradient_png(0)),
		("gfx/b.png".to_string(), gradient_png(77)),
	]));
	let registry = registry();
	let (width, height) = (96, 64);
	let kept_target = RenderTarget::new(&gpu, width, height);
	let built_target = RenderTarget::new(&gpu, width, height);
	// Our pages, and the atlases drawn whole.
	for pages in [true, false] {
		let mut kept = WorldRenderer::new(&gpu, Arc::new(Files(files.0.clone())), 1);
		let mut built = WorldRenderer::new(&gpu, Arc::new(Files(files.0.clone())), 1);
		if !pages {
			kept.draw_whole_atlases();
			built.draw_whole_atlases();
		}
		for t in 0..8 {
			let zoom = if t < 6 { 2.0 } else { 1.0 };
			let view = WorldView {
				center: [t as f32 * 0.5, 0.0],
				zoom,
				width,
				height,
			};
			let scene = scene(t);
			let none = Vec::new;
			let map = WorldMap {
				revision: 1,
				tiles: &none,
				edits: &[],
				changes: &[],
			};
			kept.begin_frame();
			let stats = kept.draw_kept(&gpu, &kept_target, &scene, &map, &registry, &view, Atlases::WaitForAll);
			built.begin_frame();
			let expected = built.draw(&gpu, &built_target, &scene, &registry, &view, Atlases::WaitForAll);
			assert_eq!(stats, expected, "step {t}, pages {pages}");
			assert!(stats.quads > 8, "step {t}: {stats:?}");
			let (a, b) = (kept_target.read(&gpu).unwrap(), built_target.read(&gpu).unwrap());
			let differ = a.rgba.chunks(4).zip(b.rgba.chunks(4)).filter(|(x, y)| x != y).count();
			assert_eq!(differ, 0, "step {t}, pages {pages}: pixels differ");
		}
	}
}
