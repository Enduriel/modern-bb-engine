//! The system cursor pictures (`cursor_image`) show what the software cursor draws: for
//! every frame of every `cursor_*` sprite of the install, the picture laid over black
//! with its hotspot at the mouse is the frame the renderer draws there. Skips without
//! the Steam install or a GPU adapter.

use std::path::Path;
use std::sync::Arc;

use bb_assets::{CursorImage, Image, decode_png};
use bb_engine::DatArchive;
use bb_engine::assets::load_sprites;
use bb_engine::files::GameFiles;
use bb_engine::harness::{self, STEAM_DATA_DIR};
use bb_render::cursor_image::cursor_frames;
use bb_render::{Gpu, RenderTarget, WorldRenderer};

const SIZE: u32 = 128;
/// The mouse's pixel.
const AT: (i64, i64) = (61, 70);

/// `image` over black with its hotspot at [`AT`], as the target's pixels.
fn laid_over_black(image: &CursorImage) -> Vec<u8> {
	let mut out = vec![0u8; (SIZE * SIZE * 4) as usize];
	for i in (3..out.len()).step_by(4) {
		out[i] = 255;
	}
	let (left, top) = (AT.0 - i64::from(image.hotspot.0), AT.1 - i64::from(image.hotspot.1));
	for y in 0..image.height {
		for x in 0..image.width {
			let (tx, ty) = (left + i64::from(x), top + i64::from(y));
			let from = ((y * image.width + x) * 4) as usize;
			let to = ((ty as u32 * SIZE + tx as u32) * 4) as usize;
			let a = u32::from(image.rgba[from + 3]);
			for c in 0..3 {
				out[to + c] = ((u32::from(image.rgba[from + c]) * a + 127) / 255) as u8;
			}
		}
	}
	out
}

#[test]
fn cursor_pictures_show_what_the_software_cursor_draws() {
	if !Path::new(STEAM_DATA_DIR).exists() {
		bb_engine::harness::skip(format!("{STEAM_DATA_DIR} not present"));
		return;
	}
	let gpu = match Gpu::headless() {
		Ok(gpu) => gpu,
		Err(e) => {
			bb_engine::harness::skip(&e);
			return;
		}
	};
	let paths = harness::data_archives(Path::new(STEAM_DATA_DIR)).expect("archives");
	let mut dats: Vec<DatArchive> = paths.iter().map(|p| DatArchive::open(p).unwrap()).collect();
	let registry = load_sprites(&mut dats).registry;
	let files = Arc::new(GameFiles::open(&paths).expect("files"));
	let mut renderer = WorldRenderer::new(&gpu, files.clone(), 2);
	let target = RenderTarget::new(&gpu, SIZE, SIZE);
	let names: Vec<String> = registry
		.sheets()
		.iter()
		.flat_map(|s| s.sheet.sprites.iter().map(|sp| sp.name.clone()))
		.filter(|n| n.starts_with("cursor_"))
		.collect();
	assert!(names.len() >= 13, "vanilla's cursors: {names:?}");
	let mut atlas: Option<(String, Image)> = None;
	let mut checked = 0;
	for name in &names {
		let (sheet, sprite) = registry.get(name).expect("listed");
		let path = &sheet.sheet.texture;
		if atlas.as_ref().is_none_or(|(p, _)| p != path) {
			let bytes = files.read(path).expect("the atlas");
			atlas = Some((path.clone(), decode_png(&bytes).expect("decodes")));
		}
		let frames = cursor_frames(sprite, &atlas.as_ref().unwrap().1).unwrap_or_else(|| panic!("{name}: no pictures"));
		for (i, image) in frames.iter().enumerate() {
			// Into the middle of frame `i`'s time.
			let since = (i as f32 + 0.5) * sprite.frame_time_ms as f32 / 1000.0;
			let at = [AT.0 as f32 - SIZE as f32 * 0.5, SIZE as f32 * 0.5 - AT.1 as f32];
			let drawn = loop {
				target.clear(&gpu);
				let stats = renderer.draw_cursor(&gpu, &target, &registry, name, since, at, 1.0);
				if stats.waiting == 0 {
					break target.read(&gpu).expect("read back");
				}
				std::thread::sleep(std::time::Duration::from_millis(5));
			};
			let expected = laid_over_black(image);
			let worst = drawn.rgba.iter().zip(&expected).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
			assert!(worst <= 1, "{name} frame {i}: differs by up to {worst} from the software cursor");
			checked += 1;
		}
	}
	println!("{checked} cursor frames match");
}
