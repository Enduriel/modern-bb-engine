//! The UI page over a frame: premultiplied colours blend over what the game drew, a
//! texel a pixel from the frame's top-left (the page is painted at the screen's
//! resolution at any UI scale), and the game shows past its edge; a paint changes only
//! the parts it says changed; the page reads back. Skips without a GPU that can take
//! the browser's paints.

use bb_assets::Image;
use bb_render::{Gpu, RenderTarget, UiLayer, UiPage, UiRegion};

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

/// A GPU and a page on it, or `None` (skip) without one that can take the browser's
/// paints.
fn page_gpu() -> Option<(Gpu, UiPage)> {
	let gpu = Gpu::headless_with(UiPage::FEATURES).map_err(|e| bb_engine::harness::skip(&e)).ok()?;
	let page = UiPage::new(&gpu).map_err(|e| bb_engine::harness::skip(&e)).ok()?;
	Some((gpu, page))
}

/// All of a `width` × `height` page.
fn whole(width: u32, height: u32) -> [UiRegion; 1] {
	[UiRegion { x: 0, y: 0, width, height }]
}

#[test]
fn the_page_blends_over_the_frame() {
	let Some((gpu, page)) = page_gpu() else { return };
	let target = blue_target(&gpu, 3, 1);
	let mut layer = UiLayer::new(&gpu, page.clone());
	layer.draw(&gpu, &target);
	assert_eq!(pixel(&target.read(&gpu).unwrap(), 0, 0), BLUE, "nothing before a frame");

	// BGRA, premultiplied: opaque red, transparent, half-covering green.
	let pixels = [[0, 0, 255, 255], [0, 0, 0, 0], [0, 128, 0, 128]].concat();
	page.write_for_tests(3, 1, &pixels, &whole(3, 1));
	layer.draw(&gpu, &target);
	let image = target.read(&gpu).unwrap();
	assert_eq!(pixel(&image, 0, 0), [255, 0, 0, 255]);
	assert_eq!(pixel(&image, 1, 0), BLUE, "the game shows through");
	assert!(close(pixel(&image, 2, 0), [0, 128, 127, 255]), "{:?}", pixel(&image, 2, 0));
}

#[test]
fn the_page_is_drawn_a_texel_a_pixel() {
	let Some((gpu, page)) = page_gpu() else { return };
	// A page a pixel short of the frame both ways (UI scale 1.5 on a 2560 screen paints
	// 1706 × 1.5 = 2559): it isn't stretched, and the game shows past it.
	let target = blue_target(&gpu, 3, 2);
	let mut layer = UiLayer::new(&gpu, page.clone());
	page.write_for_tests(2, 1, &[[0, 0, 255, 255], [0, 255, 0, 255]].concat(), &whole(2, 1));
	layer.draw(&gpu, &target);
	let image = target.read(&gpu).unwrap();
	assert_eq!(pixel(&image, 0, 0), [255, 0, 0, 255]);
	assert_eq!(pixel(&image, 1, 0), [0, 255, 0, 255]);
	assert_eq!(pixel(&image, 2, 0), BLUE, "past the page's edge");
	for x in 0..3 {
		assert_eq!(pixel(&image, x, 1), BLUE, "below the page");
	}
}

#[test]
fn a_paint_changes_only_what_it_says_changed() {
	let Some((gpu, page)) = page_gpu() else { return };
	let (red, green) = ([0, 0, 255, 255], [0, 255, 0, 255]);
	let target = blue_target(&gpu, 4, 3);
	let mut layer = UiLayer::new(&gpu, page.clone());
	// The first paint takes the whole frame, whatever regions it names.
	let whole_red: Vec<u8> = red.repeat(12);
	page.write_for_tests(4, 3, &whole_red, &[UiRegion { x: 0, y: 0, width: 1, height: 1 }]);
	assert_eq!(page.copy_stats().pixels, 4 * 3);
	// Now all green, but only a 2x2 square at (1, 1) said to have changed (and a region
	// reaching past the frame, clipped to it): the rest stays red.
	let whole_green: Vec<u8> = green.repeat(12);
	let regions = [UiRegion { x: 1, y: 1, width: 2, height: 2 }, UiRegion { x: 3, y: 2, width: 9, height: 9 }];
	page.write_for_tests(4, 3, &whole_green, &regions);
	let stats = page.copy_stats();
	assert_eq!((stats.paints, stats.rects, stats.pixels), (2, 3, 4 * 3 + 2 * 2 + 1));
	layer.draw(&gpu, &target);
	let image = target.read(&gpu).unwrap();
	for y in 0..3 {
		for x in 0..4 {
			let changed = (1..3).contains(&x) && (1..3).contains(&y) || (x, y) == (3, 2);
			let want = if changed { [0, 255, 0, 255] } else { [255, 0, 0, 255] };
			assert_eq!(pixel(&image, x, y), want, "({x}, {y})");
		}
	}
	// Read back: the page's own BGRA, any region, clipped to the page.
	let (region, bgra) = page.read(UiRegion { x: 2, y: 1, width: 5, height: 1 }).unwrap().unwrap();
	assert_eq!(region, UiRegion { x: 2, y: 1, width: 2, height: 1 });
	assert_eq!(bgra, [green, red].concat());
	assert!(page.read(UiRegion { x: 4, y: 0, width: 1, height: 1 }).unwrap().is_none(), "off the page");
	// Another size: the whole new frame.
	page.write_for_tests(2, 2, &red.repeat(4), &[UiRegion { x: 0, y: 0, width: 1, height: 1 }]);
	assert_eq!(page.size(), Some((2, 2)));
	let (_, bgra) = page.read(UiRegion { x: 0, y: 0, width: 2, height: 2 }).unwrap().unwrap();
	assert_eq!(bgra, red.repeat(4));
}
