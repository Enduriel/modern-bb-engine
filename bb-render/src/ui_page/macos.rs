//! macOS: the browser's paints as IOSurfaces, imported through Metal
//! (`newTextureWithDescriptor:iosurface:plane:`, then wgpu-hal's `texture_from_raw`).
//!
//! Chromium's GPU process (ANGLE on Metal) draws each paint into an IOSurface from a
//! small pool and hands it over in CEF's callback. The surface is wrapped as a Metal
//! texture of ours that shares its memory (no copy), and the dirty rectangles are blitted
//! into the page's texture on our queue. Chromium has finished drawing a surface when it
//! hands it over (its capture path waits for the GPU work first). CEF takes the surface
//! back for its pool when the callback returns, so the callback waits for the copy's
//! command buffer, as on Linux. On macOS that callback runs on the game's thread (CEF's
//! UI thread is the main one): the wait (Metal's completion latency, ~0.8–1 ms even for a
//! 64 px copy, up to ~3 ms for a whole 2560x1358 page on an M4) is part of the game's
//! frame. It blocks; it costs no CPU.
//!
//! Each surface is wrapped once and kept while Chromium keeps handing it over
//! ([`Imports`]).
//!
//! The Objective-C calls go through `objc2`'s `msg_send!` alone: the Metal types come from
//! wgpu-hal's signatures, so no binding crate is named here.

use std::ffi::c_void;
use std::time::Instant;

use objc2::encode::{Encoding, RefEncode};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2::msg_send;

use super::{Region, UiPage};
use crate::gpu::Gpu;

/// `MTLPixelFormatBGRA8Unorm`.
const BGRA8_UNORM: usize = 80;
/// `MTLTextureUsageShaderRead`: a blit source needs no more.
const USAGE_SHADER_READ: usize = 1;
/// `MTLStorageModeManaged`, what IOSurface-backed textures use (as Chromium does).
const STORAGE_MANAGED: usize = 1;

#[link(name = "IOSurface", kind = "framework")]
unsafe extern "C" {
	/// The surface's system-wide ID, unique while it exists.
	fn IOSurfaceGetID(surface: *mut c_void) -> u32;
}

/// An `IOSurfaceRef`'s target, for its Objective-C type encoding (`^{__IOSurface=}`).
#[repr(C)]
struct IOSurfaceTarget {
	_private: [u8; 0],
}

// SAFETY: the encoding of `IOSurfaceRef`, `struct __IOSurface *`.
unsafe impl RefEncode for IOSurfaceTarget {
	const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("__IOSurface", &[]));
}

/// One paint's buffer, as the browser describes it. Valid only while its paint's
/// callback runs.
#[derive(Clone, Copy, Debug)]
pub struct IOSurface {
	/// The `IOSurfaceRef`.
	pub surface: *mut c_void,
	pub width: u32,
	pub height: u32,
	/// BGRA byte order (else RGBA).
	pub bgra: bool,
}

/// Whether `gpu` can import IOSurfaces: a Metal device.
pub(super) fn supported(gpu: &Gpu) -> Result<(), String> {
	// SAFETY: only asks whether the device is Metal; nothing is done with it.
	let metal = unsafe { gpu.device.as_hal::<wgpu::hal::api::Metal>() }.is_some();
	if !metal {
		return Err(format!(
			"the UI page needs a Metal GPU device to take the browser's paints, and {} isn't one",
			gpu.adapter
		));
	}
	Ok(())
}

impl UiPage {
	/// Copy the parts `rects` of the browser's paint `buffer` into the page's texture
	/// (all of it on the first paint and after a resize), GPU to GPU, and wait for the
	/// copy: call it from the paint's callback, which must not return before.
	pub fn copy_iosurface(&self, buffer: &IOSurface, rects: &[Region]) -> Result<(), String> {
		let result = self.import_iosurface_and_copy(buffer, rects);
		if result.is_err() {
			self.failed();
		}
		result
	}

	fn import_iosurface_and_copy(&self, buffer: &IOSurface, rects: &[Region]) -> Result<(), String> {
		if !buffer.bgra {
			return Err("the browser painted RGBA; the UI page takes BGRA".into());
		}
		if buffer.width == 0 || buffer.height == 0 {
			return Err("the browser painted an empty frame".into());
		}
		if buffer.surface.is_null() {
			return Err("the paint has no IOSurface".into());
		}
		let started = Instant::now();
		let source = {
			let mut imports = self.inner.imports.lock().unwrap_or_else(|e| e.into_inner());
			match imports.get(buffer) {
				Some(texture) => texture,
				None => {
					let texture = self.import_iosurface(buffer)?;
					imports.keep(buffer, texture.clone());
					texture
				}
			}
		};
		self.stats_mut().import += started.elapsed();
		// The surface goes back to the browser after the copy finished.
		self.copy_from(&source, rects)
	}

	/// `buffer`'s surface as a texture of ours that shares its memory.
	fn import_iosurface(&self, buffer: &IOSurface) -> Result<wgpu::Texture, String> {
		let size = wgpu::Extent3d { width: buffer.width, height: buffer.height, depth_or_array_layers: 1 };
		let device = &self.inner.device;
		// SAFETY: the device is Metal (`UiPage::new` checked); the descriptor's selectors
		// and argument types are MTLTextureDescriptor's (NSUInteger each), and the
		// surface is the browser's IOSurfaceRef of `width` × `height` BGRA pixels, valid
		// during this call; the Metal texture made from it retains it.
		let hal_texture = unsafe {
			let hal_device = device.as_hal::<wgpu::hal::api::Metal>().ok_or("the GPU device isn't Metal")?;
			let class = AnyClass::get(c"MTLTextureDescriptor").ok_or("no MTLTextureDescriptor class")?;
			let descriptor: Retained<AnyObject> = msg_send![class, new];
			let _: () = msg_send![&*descriptor, setPixelFormat: BGRA8_UNORM];
			let _: () = msg_send![&*descriptor, setWidth: buffer.width as usize];
			let _: () = msg_send![&*descriptor, setHeight: buffer.height as usize];
			let _: () = msg_send![&*descriptor, setUsage: USAGE_SHADER_READ];
			let _: () = msg_send![&*descriptor, setStorageMode: STORAGE_MANAGED];
			let surface = buffer.surface.cast::<IOSurfaceTarget>();
			let raw_device = hal_device.raw_device();
			let texture: Option<Retained<_>> =
				msg_send![&**raw_device, newTextureWithDescriptor: &*descriptor, iosurface: surface, plane: 0usize];
			let texture = texture.ok_or("Metal couldn't make a texture of the paint's IOSurface")?;
			let texture_type = msg_send![&*texture, textureType];
			wgpu::hal::metal::Device::texture_from_raw(
				texture,
				super::FORMAT,
				texture_type,
				1,
				1,
				wgpu::hal::CopyExtent { width: buffer.width, height: buffer.height, depth: 1 },
				None,
			)
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
		// SAFETY: made on this device from the descriptor above; the browser writes it
		// before each paint that hands it over.
		Ok(unsafe {
			device.create_texture_from_hal::<wgpu::hal::api::Metal>(hal_texture, &descriptor, wgpu::TextureUses::UNINITIALIZED)
		})
	}
}

/// The browser's surfaces imported so far, by their IOSurface ID: Chromium paints into
/// a small pool of them, and wrapping one costs ~60–80 µs a paint where finding it costs
/// ~1. Each paint hands over a new `IOSurfaceRef` (looked up from a Mach port), so the
/// pointer can't tell them apart; the ID can, and a kept texture retains its surface, so
/// its ID isn't reused for another while it is here. The texture shares the surface's
/// memory, so it shows each new paint. Kept: the [`MAX_IMPORTS`] latest of the current size; a paint of
/// another size (a resize: a new pool) drops the rest.
#[derive(Default)]
pub(super) struct Imports {
	textures: Vec<(u32, wgpu::Texture)>,
}

/// More than Chromium's pool (3), so a steady pool is never re-imported.
const MAX_IMPORTS: usize = 4;

impl Imports {
	fn get(&mut self, buffer: &IOSurface) -> Option<wgpu::Texture> {
		let fits = |t: &wgpu::Texture| (t.width(), t.height()) == (buffer.width, buffer.height);
		self.textures.retain(|(_, t)| fits(t));
		let id = surface_id(buffer);
		let at = self.textures.iter().position(|(surface, _)| *surface == id)?;
		// The latest used last.
		let entry = self.textures.remove(at);
		let texture = entry.1.clone();
		self.textures.push(entry);
		Some(texture)
	}

	fn keep(&mut self, buffer: &IOSurface, texture: wgpu::Texture) {
		if self.textures.len() >= MAX_IMPORTS {
			self.textures.remove(0);
		}
		self.textures.push((surface_id(buffer), texture));
	}
}

fn surface_id(buffer: &IOSurface) -> u32 {
	// SAFETY: the browser's IOSurfaceRef, valid during the paint's call.
	unsafe { IOSurfaceGetID(buffer.surface) }
}
