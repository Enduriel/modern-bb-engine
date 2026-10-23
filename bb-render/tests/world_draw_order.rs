//! What draws over what on the world map, as in the original (disassembled and
//! screenshotted, `squirrel_bb`'s `world_entities::draw_key`): a party is
//! drawn over every settlement, location and tree; a settlement sorts among the map's
//! details by its key (its `y` less 20), so a tree a little south of it is behind it and
//! one further south in front; its banner, which keeps its screen size, 2000 lower, is
//! over both. Both ways `bb` draws: the map kept on the GPU with the moving layer kept
//! too, and a scene drawn whole. Skips when no GPU adapter is available.

use std::collections::HashMap;
use std::sync::Arc;

use bb_assets::{BrushFrame, BrushSheet, BrushSprite, FileSource, SheetHeader, SpriteRegistry, name_hash};
use bb_render::world::WorldView;
use bb_render::{Atlases, Gpu, RenderTarget, WorldMap, WorldRenderer};
use squirrel_bb::scene::{SceneDetail, SceneEntity, SceneSprite, SceneTile, WorldScene};

struct Files(HashMap<String, Vec<u8>>);

impl FileSource for Files {
	fn read(&self, path: &str) -> Option<Vec<u8>> {
		self.0.get(path).cloned()
	}
}

const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const YELLOW: [u8; 4] = [255, 255, 0, 255];

/// A 2×2 PNG of one colour.
fn solid_png(rgba: [u8; 4]) -> Vec<u8> {
	let mut out = Vec::new();
	let mut encoder = png::Encoder::new(&mut out, 2, 2);
	encoder.set_color(png::ColorType::Rgba);
	encoder.set_depth(png::BitDepth::Eight);
	let mut writer = encoder.write_header().unwrap();
	writer.write_image_data(&[rgba; 4].concat()).unwrap();
	writer.finish().unwrap();
	out
}

/// The brushes, each its own sheet of one colour: a settlement (`town`, reaching far
/// below its anchor, as the settlements' pictures do), a party, a tree and a banner. All
/// with a depth bias, which an entity's sprites don't add.
fn registry() -> SpriteRegistry {
	let mut registry = SpriteRegistry::new();
	for (name, rect, depth_bias) in [
		("town", [-12.0, 12.0, -30.0, 12.0], 50),
		("party", [-4.0, 4.0, -4.0, 4.0], -50),
		("tree", [-4.0, 4.0, -4.0, 4.0], 0),
		("banner", [-2.0, 2.0, -2.0, 2.0], 50),
	] {
		let sprite = BrushSprite {
			hash: name_hash(name.as_bytes()),
			name: name.into(),
			material: 0,
			width: 2,
			height: 2,
			offset_x: 0,
			offset_y: 0,
			depth_bias,
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
				rect,
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
		registry.add_sheet(
			&format!("brushes/{name}.brush"),
			BrushSheet {
				texture: format!("gfx/{name}.png"),
				header,
				sprites: vec![sprite],
			},
		);
	}
	registry
}

fn sprite(name: &str, brush: &str, offset: (f64, f64), scaling: bool) -> SceneSprite {
	SceneSprite {
		name: name.into(),
		brush: brush.into(),
		color: 0xffff_ffff,
		scale: 1.0,
		visible: true,
		offset,
		flipped: false,
		rotation: 0.0,
		ignore_ambient: false,
		scaling,
		colorization: true,
		frame: 0,
	}
}

/// A settlement at the origin (spawned there: key `0 - 20`), its banner over the tree
/// south of it; a party north of it, overlapping it.
fn entities() -> Vec<SceneEntity> {
	vec![
		SceneEntity {
			id: 1,
			pos: (0.0, 0.0),
			shown: true,
			in_details: true,
			key: -20,
			sprites: vec![sprite("body", "town", (0.0, 0.0), true), sprite("location_banner", "banner", (-6.0, -22.0), false)],
			labels: Vec::new(),
		},
		SceneEntity {
			id: 2,
			pos: (-6.0, 8.0),
			shown: true,
			in_details: false,
			key: 8,
			sprites: vec![sprite("body", "party", (0.0, 0.0), true)],
			labels: Vec::new(),
		},
	]
}

/// The map: a tree 10 south of the settlement (keyed above it) and one 26 south (keyed
/// below it), on one tile without terrain.
fn tiles() -> Vec<SceneTile> {
	let tree = |id: i64, pos: (f32, f32)| SceneDetail {
		id,
		brush: "tree".into(),
		pos,
		z: pos.1 as i32,
		..SceneDetail::default()
	};
	vec![SceneTile {
		col: 0,
		row: 0,
		pos: (0.0, 0.0),
		terrain: 2,
		discovered: true,
		brush: "".into(),
		flipped: false,
		details: vec![tree(10, (6.0, -10.0)), tree(11, (-6.0, -26.0))],
	}]
}

#[test]
fn parties_draw_over_settlements_which_sort_among_the_details() {
	let gpu = match Gpu::headless() {
		Ok(gpu) => gpu,
		Err(e) => {
			bb_engine::harness::skip(&e);
			return;
		}
	};
	let files: HashMap<String, Vec<u8>> = [("town", RED), ("party", GREEN), ("tree", BLUE), ("banner", YELLOW)]
		.into_iter()
		.map(|(name, colour)| (format!("gfx/{name}.png"), solid_png(colour)))
		.collect();
	let registry = registry();
	let (width, height) = (64, 64);
	let view = WorldView {
		fit: 1.0,
		center: [0.0, 0.0],
		zoom: 1.0,
		width,
		height,
	};
	let target = RenderTarget::new(&gpu, width, height);
	// The colour at world point (x, y): y up, the view centred on the origin.
	let at = |image: &bb_assets::Image, (x, y): (f32, f32)| {
		let (px, py) = ((x + 32.0) as usize, (32.0 - y) as usize);
		let i = (py * width as usize + px) * 4;
		<[u8; 4]>::try_from(&image.rgba[i..i + 4]).unwrap()
	};
	let check = |image: &bb_assets::Image, how: &str| {
		assert_eq!(at(image, (-6.5, 8.5)), GREEN, "{how}: the party north of the settlement is over it");
		assert_eq!(at(image, (0.5, 0.5)), RED, "{how}: the settlement");
		assert_eq!(at(image, (6.5, -9.5)), RED, "{how}: the settlement over the tree 10 south of it");
		assert_eq!(at(image, (-6.5, -26.5)), BLUE, "{how}: the tree 26 south over the settlement");
		assert_eq!(at(image, (-6.5, -23.0)), YELLOW, "{how}: the banner over that tree");
	};

	// As `bb` draws: the map kept, the moving layer kept.
	let mut kept = WorldRenderer::new(&gpu, Arc::new(Files(files.clone())), 1);
	let scene = WorldScene {
		entities: entities(),
		fog_of_war: false,
		..WorldScene::default()
	};
	let map = WorldMap {
		revision: 1,
		tiles: &tiles,
		edits: &[],
		changes: &[],
	};
	for frame in 0..2 {
		kept.begin_frame();
		kept.draw_kept(&gpu, &target, &scene, &map, &registry, &view, Atlases::WaitForAll);
		check(&target.read(&gpu).unwrap(), &format!("kept, frame {frame}"));
	}

	// A scene drawn whole.
	let mut whole = WorldRenderer::new(&gpu, Arc::new(Files(files)), 1);
	let scene = WorldScene {
		tiles: tiles(),
		..scene
	};
	whole.begin_frame();
	whole.draw(&gpu, &target, &scene, &registry, &view, Atlases::WaitForAll);
	check(&target.read(&gpu).unwrap(), "drawn whole");
}
