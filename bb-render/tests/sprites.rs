//! The sprite program on a real GPU: a 2 x 2 test atlas drawn through a frame quad
//! lands right side up (atlases are stored upside down, as bbrusher packs them), and
//! alpha blends over the background. Skips when no GPU adapter is available.

use bb_assets::{BrushFrame, Image};
use bb_render::sprite::{SceneUniforms, SpriteRenderer};
use bb_render::world::frame_quad;
use bb_render::{Gpu, GpuTexture, RenderTarget};

const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const CLEAR: [u8; 4] = [255, 255, 255, 0];

fn pixel(image: &Image, x: u32, y: u32) -> [u8; 4] {
	let i = ((y * image.width + x) * 4) as usize;
	image.rgba[i..i + 4].try_into().unwrap()
}

#[test]
fn atlases_draw_right_side_up_and_blend() {
	let gpu = match Gpu::headless() {
		Ok(gpu) => gpu,
		Err(e) => {
			eprintln!("skipping: {e}");
			return;
		}
	};
	// As stored: the first row is the bottom of the picture.
	let atlas = Image {
		width: 2,
		height: 2,
		rgba: [RED, GREEN, BLUE, CLEAR].concat(),
	};
	let texture = GpuTexture::upload(&gpu, "test atlas", &atlas);
	// A 2 x 2 world quad centred on the origin, covering the whole atlas, drawn one
	// texel per pixel so every pixel centre is a texel centre (no filtering blend).
	let frame = BrushFrame {
		name: String::new(),
		uv: [0.0, 1.0, 0.0, 1.0],
		rect: [-1.0, 1.0, -1.0, 1.0],
	};
	let quad = frame_quad(&frame, [0.0, 0.0], 1.0, false, 255.0);

	let renderer = SpriteRenderer::new(&gpu);
	let target = RenderTarget::new(&gpu, 2, 2);
	let scene = SceneUniforms::centered([0.0, 0.0], 2.0, 2.0);
	renderer.draw(
		&gpu,
		&target,
		&scene,
		Some([0.0, 0.0, 0.0, 1.0]),
		&[quad],
		&[&texture],
	);
	let image = target.read(&gpu).expect("read back");

	// The atlas's second row (blue, clear) shows on top, its first (red, green) below.
	assert_eq!(pixel(&image, 0, 0), [0, 0, 255, 255], "top left is blue");
	assert_eq!(
		pixel(&image, 1, 1),
		[0, 255, 0, 255],
		"bottom right is green"
	);
	assert_eq!(pixel(&image, 0, 1), [255, 0, 0, 255], "bottom left is red");
	// The clear texel leaves the black background (alpha 0 contributes nothing).
	assert_eq!(
		&pixel(&image, 1, 0)[..3],
		&[0, 0, 0],
		"top right shows the background"
	);
}
