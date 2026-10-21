//! The page's frame on the GPU: bb-render's [`UiPage`] as the browser host's paint sink.
//! Each paint's GPU buffer is copied into it as it lands (on CEF's thread), and the game
//! draws it over its frames ([`UiLayer`](bb_render::UiLayer)).

use std::sync::Arc;

use bb_render::{UiPage, UiRegion};
use bb_ui::host::{PageSink, PaintBuffer, PaintRect, SharedPaint};

struct GpuPage(UiPage);

fn region(r: PaintRect) -> UiRegion {
	UiRegion { x: r.x, y: r.y, width: r.width, height: r.height }
}

/// The paint's changed parts, as the page takes them.
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
fn dirty(paint: &SharedPaint) -> Vec<UiRegion> {
	paint.dirty.iter().map(|r| region(*r)).collect()
}

impl PageSink for GpuPage {
	fn paint(&self, paint: &SharedPaint) -> Result<(), String> {
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
				self.0.copy_dmabuf(&buffer, &dirty(paint))
			}
			#[cfg(target_os = "macos")]
			PaintBuffer::IOSurface(surface) => {
				let buffer = bb_render::IOSurface {
					surface: surface.as_ptr(),
					width: paint.width,
					height: paint.height,
					bgra: paint.bgra,
				};
				self.0.copy_iosurface(&buffer, &dirty(paint))
			}
			#[cfg(windows)]
			PaintBuffer::D3D11(texture) => {
				let texture = bb_render::D3D11Texture {
					handle: texture.as_ptr(),
					width: paint.width,
					height: paint.height,
					bgra: paint.bgra,
				};
				self.0.copy_d3d11(&texture, &dirty(paint))
			}
			#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
			PaintBuffer::Unsupported(_) => Err("no GPU path for the UI page's paints on this operating system yet".into()),
		}
	}

	fn read(&self, r: PaintRect) -> Result<Option<(PaintRect, Vec<u8>)>, String> {
		Ok(self.0.read(region(r))?.map(|(r, bgra)| (PaintRect { x: r.x, y: r.y, width: r.width, height: r.height }, bgra)))
	}

	fn browser_switches(&self) -> Vec<(&'static str, String)> {
		self.0.browser_switches()
	}
}

/// `page` as the host's paint sink.
pub fn sink(page: UiPage) -> Arc<dyn PageSink> {
	Arc::new(GpuPage(page))
}
