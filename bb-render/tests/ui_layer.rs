//! The UI page over a frame: premultiplied colours blend over what the game drew, a
//! texel a pixel from the frame's top-left (the page is painted at the screen's
//! resolution at any UI scale), and the game shows past its edge. Skips without a GPU.

use bb_assets::Image;
use bb_render::{Gpu, RenderTarget, UiLayer};

const BLUE: [u8; 4] = [0, 0, 255, 255];

fn pixel(image: &Image, x: u32, y: u32) -> [u8; 4] {
	let i = ((y * image.width + x) * 4) as usize;
	image.rgba[i..i + 4].try_into().unwrap()
}

fn close(a: [u8; 4], b: [u8; 4]) -> bool {
	a.iter().zip(b).all(|(&x, y)| x.abs_diff(y) <= 1)
}

/// A `width` × `height` target filled with blue.
fn blue_target(gpu: &Gpu, width: u32, height: u32) -> RenderTarget {
	let target = RenderTarget::new(gpu, width, height);
	let mut encoder = gpu
		.device
		.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
	encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
		label: None,
		color_attachments: &[Some(wgpu::RenderPassColorAttachment {
			view: &target.view,
			depth_slice: None,
			resolve_target: None,
			ops: wgpu::Operations {
				load: wgpu::LoadOp::Clear(wgpu::Color::BLUE),
				store: wgpu::StoreOp::Store,
			},
		})],
		depth_stencil_attachment: None,
		timestamp_writes: None,
		occlusion_query_set: None,
		multiview_mask: None,
	});
	gpu.queue.submit([encoder.finish()]);
	target
}

#[test]
fn the_page_blends_over_the_frame() {
	let gpu = match Gpu::headless() {
		Ok(gpu) => gpu,
		Err(e) => {
			eprintln!("skipping: {e}");
			return;
		}
	};
	let target = blue_target(&gpu, 3, 1);
	let mut layer = UiLayer::new(&gpu);
	layer.draw(&gpu, &target);
	assert_eq!(pixel(&target.read(&gpu).unwrap(), 0, 0), BLUE, "nothing before a frame");

	// BGRA, premultiplied: opaque red, transparent, half-covering green.
	let page = [[0, 0, 255, 255], [0, 0, 0, 0], [0, 128, 0, 128]].concat();
	layer.upload(&gpu, 3, 1, &page);
	layer.draw(&gpu, &target);
	let image = target.read(&gpu).unwrap();
	assert_eq!(pixel(&image, 0, 0), [255, 0, 0, 255]);
	assert_eq!(pixel(&image, 1, 0), BLUE, "the game shows through");
	assert!(close(pixel(&image, 2, 0), [0, 128, 127, 255]), "{:?}", pixel(&image, 2, 0));
}

#[test]
fn the_page_is_drawn_a_texel_a_pixel() {
	let gpu = match Gpu::headless() {
		Ok(gpu) => gpu,
		Err(e) => {
			eprintln!("skipping: {e}");
			return;
		}
	};
	// A page a pixel short of the frame both ways (UI scale 1.5 on a 2560 screen paints
	// 1706 × 1.5 = 2559): it isn't stretched, and the game shows past it.
	let target = blue_target(&gpu, 3, 2);
	let mut layer = UiLayer::new(&gpu);
	layer.upload(&gpu, 2, 1, &[[0, 0, 255, 255], [0, 255, 0, 255]].concat());
	layer.draw(&gpu, &target);
	let image = target.read(&gpu).unwrap();
	assert_eq!(pixel(&image, 0, 0), [255, 0, 0, 255]);
	assert_eq!(pixel(&image, 1, 0), [0, 255, 0, 255]);
	assert_eq!(pixel(&image, 2, 0), BLUE, "past the page's edge");
	for x in 0..3 {
		assert_eq!(pixel(&image, x, 1), BLUE, "below the page");
	}
}
