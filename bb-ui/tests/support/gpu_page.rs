//! The page's frame on the GPU for the tests and tools: bb-render's `UiPage` as the
//! host's paint sink (the game does the same in bb-app's `front`).

use std::sync::Arc;

use bb_render::{Gpu, UiPage, UiRegion};
use bb_ui::host::{PageSink, PaintBuffer, PaintRect, SharedPaint};

struct GpuPage(UiPage);

fn region(r: PaintRect) -> UiRegion {
	UiRegion { x: r.x, y: r.y, width: r.width, height: r.height }
}

impl PageSink for GpuPage {
	fn paint(&self, paint: &SharedPaint) -> Result<(), String> {
		let rects: Vec<UiRegion> = paint.dirty.iter().map(|r| region(*r)).collect();
		match paint.buffer {
			#[cfg(target_os = "linux")]
			PaintBuffer::DmaBuf { planes, modifier } => {
				let [plane] = planes else {
					return Err(format!("a paint in {} planes; the page takes one", planes.len()));
				};
				let buffer = bb_render::DmaBuf {
					fd: plane.fd,
					stride: plane.stride,
					offset: plane.offset,
					modifier,
					width: paint.width,
					height: paint.height,
					bgra: paint.bgra,
				};
				self.0.copy_dmabuf(&buffer, &rects)
			}
			#[cfg(target_os = "macos")]
			PaintBuffer::IOSurface(surface) => {
				let buffer = bb_render::IOSurface {
					surface: surface.as_ptr(),
					width: paint.width,
					height: paint.height,
					bgra: paint.bgra,
				};
				self.0.copy_iosurface(&buffer, &rects)
			}
		}
	}

	fn read(&self, r: PaintRect) -> Result<Option<(PaintRect, Vec<u8>)>, String> {
		Ok(self.0.read(region(r))?.map(|(r, bgra)| (PaintRect { x: r.x, y: r.y, width: r.width, height: r.height }, bgra)))
	}
}

/// A GPU device that can take the page's paints, and a sink for them on it.
pub fn sink() -> Result<Arc<dyn PageSink>, String> {
	let gpu = Gpu::headless_with(UiPage::FEATURES)?;
	Ok(Arc::new(GpuPage(UiPage::new(&gpu)?)))
}
