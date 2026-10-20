//! Where the page's paints go: Chromium paints the page on the GPU and hands each paint
//! over as one of its own GPU buffers (CEF's accelerated paint, a shared texture). The
//! host passes it on, as plain data, to the [`PageSink`] its owner installs, which
//! copies what changed into a texture of its own (bb-render's `UiPage`). bb-ui itself
//! never touches the GPU.
//!
//! Only Linux's buffers (DMA-BUFs) so far; Windows' D3D11 shared handles and macOS's
//! IOSurfaces are still to come.

#[cfg(target_os = "linux")]
use std::os::fd::BorrowedFd;

use super::damage::PaintRect;

/// One plane of a DMA-BUF paint. The fd is the browser's: valid only during the
/// paint's call.
#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug)]
pub struct DmaBufPlane<'a> {
	pub fd: BorrowedFd<'a>,
	pub stride: u32,
	pub offset: u64,
	pub size: u64,
}

/// The GPU buffer a paint is in.
#[derive(Clone, Copy, Debug)]
pub enum PaintBuffer<'a> {
	/// Linux: a DMA-BUF, with its DRM format modifier (`0x00ffffffffffffff`, none
	/// given: the driver's implicit layout).
	#[cfg(target_os = "linux")]
	DmaBuf { planes: &'a [DmaBufPlane<'a>], modifier: u64 },
	/// No other operating system's yet.
	#[cfg(not(target_os = "linux"))]
	#[doc(hidden)]
	Unsupported(std::marker::PhantomData<&'a ()>),
}

/// A paint of the page, valid only during [`PageSink::paint`]: the browser takes the
/// buffer back when the call returns.
#[derive(Clone, Copy, Debug)]
pub struct SharedPaint<'a> {
	pub width: u32,
	pub height: u32,
	/// BGRA byte order (else RGBA). Premultiplied either way.
	pub bgra: bool,
	pub buffer: PaintBuffer<'a>,
	/// The parts that changed since the last paint, inside the frame (all of it for the
	/// first paint and after a resize).
	pub dirty: &'a [PaintRect],
}

/// The owner's side of the page's paints: the page's frame on the GPU.
pub trait PageSink: Send + Sync {
	/// Copy `paint`'s dirty rectangles into the page's frame, and finish with the
	/// buffer before returning. Runs on CEF's thread, once per paint.
	fn paint(&self, paint: &SharedPaint) -> Result<(), String>;

	/// The page's frame in `region` (clipped to it): BGRA, premultiplied, rows top to
	/// bottom, with the region as clipped; `None` before the first paint or if nothing
	/// of `region` is on the page. For checking the page's pixels (is the cursor over
	/// the UI) and screenshots: a read from the GPU, so it waits for it.
	fn read(&self, region: PaintRect) -> Result<Option<(PaintRect, Vec<u8>)>, String>;
}
