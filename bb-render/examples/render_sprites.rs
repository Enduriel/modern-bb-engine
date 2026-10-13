//! Render named sprites from an install's brushes side by side into a PNG, to check
//! atlas orientation, anchors and blending by eye.
//!
//! ```text
//! cargo run -p bb-render --example render_sprites -- OUT.png NAME...
//! ```

use std::path::Path;

use bb_assets::load_image;
use bb_engine::DatArchive;
use bb_engine::assets::load_sprites;
use bb_engine::files::GameFiles;
use bb_engine::harness::{self, STEAM_DATA_DIR};
use bb_render::sprite::{Quad, SceneUniforms, SpriteRenderer};
use bb_render::world::frame_quad;
use bb_render::{Gpu, GpuTexture, RenderTarget, save_png};

fn main() {
	let mut args = std::env::args().skip(1);
	let out = args.next().expect("usage: render_sprites OUT.png NAME...");
	let names: Vec<String> = args.collect();
	let paths = harness::data_archives(Path::new(STEAM_DATA_DIR)).expect("list archives");
	let mut archives: Vec<DatArchive> =
		paths.iter().map(|p| DatArchive::open(p).unwrap()).collect();
	let registry = load_sprites(&mut archives).registry;
	let files = GameFiles::open(&paths).expect("open archives");

	let gpu = Gpu::headless().expect("gpu");
	eprintln!("adapter: {}", gpu.adapter);
	let renderer = SpriteRenderer::new(&gpu);

	let mut textures: Vec<GpuTexture> = Vec::new();
	let mut texture_paths: Vec<String> = Vec::new();
	let mut quads = Vec::new();
	let mut x = 0.0f32;
	let mut height = 0.0f32;
	for name in &names {
		let (sheet, sprite) = registry
			.get(name)
			.unwrap_or_else(|| panic!("no sprite {name}"));
		let texture = match texture_paths.iter().position(|p| *p == sheet.sheet.texture) {
			Some(i) => i,
			None => {
				let image = load_image(&files, &sheet.sheet.texture).expect("atlas");
				textures.push(GpuTexture::upload(&gpu, &sheet.sheet.texture, &image));
				texture_paths.push(sheet.sheet.texture.clone());
				textures.len() - 1
			}
		};
		let frame = &sprite.frames[0];
		let [left, right, top, bottom] = frame.rect;
		let width = right - left;
		eprintln!(
			"{name}: {}x{} offset ({}, {}) rect {:?} uv {:?}",
			sprite.width, sprite.height, sprite.offset_x, sprite.offset_y, frame.rect, frame.uv
		);
		let anchor = [x + 8.0 - left, 0.0];
		quads.push(Quad {
			texture,
			flags: 1,
			..frame_quad(frame, anchor, 1.0, false, 255.0)
		});
		x += width + 16.0;
		height = height.max(bottom - top);
	}
	let (w, h) = (x.ceil() as u32 + 16, height.ceil() as u32 + 16);
	let target = RenderTarget::new(&gpu, w, h);
	let scene = SceneUniforms::centered([w as f32 / 2.0, 0.0], w as f32, h as f32);
	let refs: Vec<&GpuTexture> = textures.iter().collect();
	renderer.draw(
		&gpu,
		&target,
		&scene,
		Some([0.2, 0.3, 0.4, 1.0]),
		&quads,
		&refs,
	);
	let image = target.read(&gpu).expect("read back");
	save_png(&out, &image).expect("save");
	eprintln!("wrote {out} ({w}x{h})");
}
