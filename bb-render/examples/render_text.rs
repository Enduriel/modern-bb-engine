//! Render a string with one of the game's bitmap fonts into a PNG, to check glyph
//! layout against screenshots of the original.
//!
//! ```text
//! cargo run -p bb-render --example render_text -- OUT.png FONT TEXT
//! ```
//! `FONT` is a name under `gfx/fonts/`, e.g. `cinzel_bold_20`.

use std::path::Path;

use bb_assets::{BitmapFont, load_image};
use bb_engine::files::GameFiles;
use bb_engine::harness::{self, STEAM_DATA_DIR};
use bb_render::sprite::{Quad, SceneUniforms, SpriteRenderer};
use bb_render::{Gpu, GpuTexture, RenderTarget, save_png};

fn main() {
	let mut args = std::env::args().skip(1);
	let out = args.next().expect("usage: render_text OUT.png FONT TEXT");
	let name = args.next().expect("font");
	let text = args.next().expect("text");
	let files =
		GameFiles::open(harness::data_archives(Path::new(STEAM_DATA_DIR)).unwrap()).unwrap();
	let font = BitmapFont::parse(&files.read(&format!("gfx/fonts/{name}.fnt")).expect("font"))
		.expect("parse");
	let atlas = load_image(&files, &format!("gfx/fonts/{name}.png")).expect("atlas");
	let gpu = Gpu::headless().expect("gpu");
	let texture = GpuTexture::upload(&gpu, &name, &atlas);

	let (cw, ch) = (font.cell_width as f32, font.cell_height as f32);
	let mut pen = 4.0f32;
	let mut quads = Vec::new();
	for &c in text.as_bytes() {
		if let Some([u0, u1, v0, v1]) = font.glyph_uv(c) {
			// The picture's top edge (v1) at the top of the line.
			let (top, bottom) = (ch, 0.0);
			quads.push(Quad {
				texture: 0,
				corners: [
					[pen, top],
					[pen + cw, top],
					[pen + cw, bottom],
					[pen, bottom],
				],
				uv: [[u0, v1], [u1, v1], [u1, v0], [u0, v0]],
				color: [255; 4],
				corner_colors: None,
				overlay: [0; 4],
				factor: 0.0,
				saturation: 1.0,
				flags: 1,
				level: 0,
			});
		}
		pen += f32::from(font.advances[usize::from(c)]);
	}
	let (w, h) = (pen.ceil() as u32 + 8, font.cell_height + 4);
	let target = RenderTarget::new(&gpu, w, h);
	let scene = SceneUniforms::centered([w as f32 / 2.0, ch / 2.0], w as f32, h as f32);
	SpriteRenderer::new(&gpu).draw(
		&gpu,
		&target,
		&scene,
		Some([0.15, 0.12, 0.1, 1.0]),
		&quads,
		&[&texture],
	);
	save_png(&out, &target.read(&gpu).expect("read")).expect("save");
	eprintln!(
		"wrote {out} ({w}x{h}), advance {}",
		font.text_width(text.as_bytes())
	);
}
