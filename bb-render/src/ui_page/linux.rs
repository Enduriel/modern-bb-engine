//! Linux: the browser's paints as DMA-BUFs, imported through Vulkan
//! (`VK_EXT_external_memory_dma_buf`, wgpu-hal's `texture_from_dmabuf_fd`).
//!
//! CEF 154 hands out its "mappable" shared images, GBM linear buffers, one plane of
//! BGRA: X11 reports no modifier (`DRM_FORMAT_MOD_INVALID`, the driver's implicit
//! layout, linear for these), Wayland `DRM_FORMAT_MOD_LINEAR`. Two things this doesn't
//! do yet, both for the next CEF upgrade (they need `ash`): acquire the image from
//! `VK_QUEUE_FAMILY_FOREIGN_EXT` with a layout that keeps its contents (tiled or
//! compressed modifiers, CEF ≥156), and wait on Chromium's write fence (NVIDIA attaches
//! no implicit fence to the buffer; Mesa does, so on Mesa the copy sees the finished
//! paint). Each pooled buffer is imported once and kept (`imports`), so wgpu
//! transitions it to a copy source only on its first paint, not before each; that is
//! fine for linear buffers, and the foreign-queue acquire will have to be recorded
//! before every copy.

use std::os::fd::BorrowedFd;
use std::time::Instant;

use super::imports::Key;
use super::{Region, UiPage};
use crate::gpu::Gpu;

const DRM_FORMAT_MOD_INVALID: u64 = 0x00ff_ffff_ffff_ffff;
const DRM_FORMAT_MOD_LINEAR: u64 = 0;

/// One paint's buffer, as the browser describes it. Valid only while its paint's
/// callback runs.
#[derive(Clone, Copy, Debug)]
pub struct DmaBuf<'a> {
	/// The buffer's one plane (BGRA has one).
	pub fd: BorrowedFd<'a>,
	pub stride: u32,
	pub offset: u64,
	/// The DRM format modifier (`DRM_FORMAT_MOD_INVALID`: none given).
	pub modifier: u64,
	pub width: u32,
	pub height: u32,
	/// BGRA byte order (else RGBA).
	pub bgra: bool,
}

/// Whether `gpu` can import DMA-BUFs: a Vulkan device with the extensions.
pub(super) fn supported(gpu: &Gpu) -> Result<(), String> {
	// SAFETY: only asks whether the device is Vulkan; nothing is done with it.
	let vulkan = unsafe { gpu.device.as_hal::<wgpu::hal::api::Vulkan>() }.is_some();
	if !vulkan {
		return Err(format!(
			"the UI page needs a Vulkan GPU device to take the browser's paints, and {} isn't one",
			gpu.adapter
		));
	}
	Ok(())
}

impl UiPage {
	/// Copy the parts `rects` of the browser's paint `buffer` into the page's texture
	/// (all of it on the first paint and after a resize), GPU to GPU, and wait for the
	/// copy: call it from the paint's callback, which must not return before.
	pub fn copy_dmabuf(&self, buffer: &DmaBuf, rects: &[Region]) -> Result<(), String> {
		let result = self.import_and_copy(buffer, rects);
		if result.is_err() {
			self.failed();
		}
		result
	}

	fn import_and_copy(&self, buffer: &DmaBuf, rects: &[Region]) -> Result<(), String> {
		if !buffer.bgra {
			return Err("the browser painted RGBA; the UI page takes BGRA".into());
		}
		if buffer.width == 0 || buffer.height == 0 {
			return Err("the browser painted an empty frame".into());
		}
		let key = Key::of(buffer)?;
		let (source, imported) = {
			let mut imports = self.inner.imports.lock().unwrap_or_else(|e| e.into_inner());
			imports.get(key, || self.import(buffer))?
		};
		if imported {
			self.stats_mut().imports += 1;
		}
		// Kept (see `imports`): read only now, during the paint's callback.
		self.copy_from(&source, rects)
	}

	/// Import `buffer` as a texture to copy from.
	fn import(&self, buffer: &DmaBuf) -> Result<wgpu::Texture, String> {
		let started = Instant::now();
		// Vulkan takes ownership of the fd it imports; the browser keeps its own. Ours
		// keeps the buffer alive as long as the import.
		let fd = buffer.fd.try_clone_to_owned().map_err(|e| format!("duplicating the paint's buffer: {e}"))?;
		// CEF 154's buffers are linear (see the module notes).
		let modifier = if buffer.modifier == DRM_FORMAT_MOD_INVALID { DRM_FORMAT_MOD_LINEAR } else { buffer.modifier };
		let size = wgpu::Extent3d { width: buffer.width, height: buffer.height, depth_or_array_layers: 1 };
		let hal_descriptor = wgpu::hal::TextureDescriptor {
			label: Some("browser paint"),
			size,
			mip_level_count: 1,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: super::FORMAT,
			usage: wgpu::TextureUses::COPY_SRC,
			memory_flags: wgpu::hal::MemoryFlags::empty(),
			view_formats: vec![],
		};
		let device = &self.inner.device;
		// SAFETY: the device is Vulkan (`UiPage::new` checked) with DMA-BUF import
		// (`UiPage::FEATURES`); the descriptor is the buffer the browser described, and
		// the fd is ours.
		let hal_texture = unsafe {
			let hal_device = device.as_hal::<wgpu::hal::api::Vulkan>().ok_or("the GPU device isn't Vulkan")?;
			hal_device
				.texture_from_dmabuf_fd(fd, &hal_descriptor, modifier, u64::from(buffer.stride), buffer.offset)
				.map_err(|e| format!("importing the paint's buffer: {e:?}"))?
		};
		let descriptor = wgpu::TextureDescriptor {
			label: Some("browser paint"),
			size,
			mip_level_count: 1,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: super::FORMAT,
			usage: wgpu::TextureUsages::COPY_SRC,
			view_formats: &[],
		};
		// SAFETY: made on this device from the descriptor above; the browser has
		// written it (its paint is done when the callback runs).
		let source = unsafe {
			device.create_texture_from_hal::<wgpu::hal::api::Vulkan>(hal_texture, &descriptor, wgpu::TextureUses::UNINITIALIZED)
		};
		self.stats_mut().import += started.elapsed();
		Ok(source)
	}
}
