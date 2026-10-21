//! Windows: the browser's paints as D3D11 shared textures (NT handles), opened on
//! wgpu's Direct3D 12 device (`ID3D12Device::OpenSharedHandle`, then wgpu-hal's
//! `texture_from_raw`).
//!
//! Chromium composites the page with ANGLE on D3D11 and hands each paint over as an NT
//! handle to a BGRA texture from a small pool (CEF's `shared_texture_handle`). D3D11 →
//! D3D12 sharing of NT handles is the route every Windows 10 driver supports (WDDM 2.0),
//! and Direct3D 12 is wgpu's native backend on Windows, so the game draws through it
//! there ([`crate::Gpu`] pins it). The other route, Vulkan's
//! `VK_KHR_external_memory_win32` with the D3D11 texture handle type, depends more on
//! the vendor's Vulkan driver and on a Vulkan driver being installed at all.
//!
//! Both sides must be on one GPU: Chromium is told wgpu's adapter
//! (`--use-adapter-luid`, [`adapter_switch`]), or on a laptop with two GPUs it may paint
//! on the other one, whose textures this device can't open.
//!
//! CEF's rules are Linux's and macOS's: the texture may only be used during the paint's
//! callback, and goes back to the pool when it returns, so the copy is submitted and
//! waited for inside it. CEF creates it without a keyed mutex (its header says so), so
//! there is nothing to acquire; Chromium has flushed its D3D11 work when it hands the
//! texture over.
//!
//! Each pooled texture is opened once and kept while the browser keeps handing it over
//! ([`Imports`]). Each paint comes with a handle of its own (a duplicate of the pool's,
//! closed when the callback returns), so the value can't tell textures apart: a kept
//! import holds its own duplicate, and `CompareObjectHandles` (Windows 10 1607) says
//! whether a paint's handle is the same texture. Like Linux's DMA-BUF imports, the kept
//! texture's state in wgpu is a copy source after its first paint, so later copies
//! record no barrier; whether a driver wants the shared texture taken back to
//! `D3D12_RESOURCE_STATE_COMMON` between D3D11's writes is for a run on real Windows
//! with the debug layer.

use std::ffi::c_void;
use std::time::Instant;

use windows::Win32::Foundation::{CloseHandle, CompareObjectHandles, DUPLICATE_SAME_ACCESS, DuplicateHandle, HANDLE};
use windows::Win32::Graphics::Direct3D12::{D3D12_RESOURCE_DIMENSION_TEXTURE2D, ID3D12Resource};
use windows::Win32::Graphics::Dxgi::Common::{
	DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_TYPELESS, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_B8G8R8A8_UNORM_SRGB,
};
use windows::Win32::System::Threading::GetCurrentProcess;

use super::{Region, UiPage};
use crate::gpu::Gpu;

/// One paint's texture, as the browser describes it. Valid only while its paint's
/// callback runs.
#[derive(Clone, Copy, Debug)]
pub struct D3D11Texture {
	/// The NT handle to the shared texture (CEF's `shared_texture_handle`).
	pub handle: *mut c_void,
	pub width: u32,
	pub height: u32,
	/// BGRA byte order (else RGBA).
	pub bgra: bool,
}

/// Whether `gpu` can open D3D11 shared textures: a Direct3D 12 device.
pub(super) fn supported(gpu: &Gpu) -> Result<(), String> {
	// SAFETY: only asks whether the device is Direct3D 12; nothing is done with it.
	let dx12 = unsafe { gpu.device.as_hal::<wgpu::hal::api::Dx12>() }.is_some();
	if !dx12 {
		return Err(format!(
			"the UI page needs a Direct3D 12 GPU device to take the browser's paints, and {} isn't one",
			gpu.adapter
		));
	}
	Ok(())
}

/// Chromium's switch that puts its GPU work on `device`'s adapter: `--use-adapter-luid`
/// with the adapter's LUID as `HighPart,LowPart` (Chromium's `ParseLuid`).
pub(super) fn adapter_switch(device: &wgpu::Device) -> Option<(&'static str, String)> {
	// SAFETY: the raw device is only asked for its adapter's LUID.
	let luid = unsafe {
		let hal_device = device.as_hal::<wgpu::hal::api::Dx12>()?;
		hal_device.raw_device().GetAdapterLuid()
	};
	Some(("use-adapter-luid", format!("{},{}", luid.HighPart, luid.LowPart)))
}

impl UiPage {
	/// Copy the parts `rects` of the browser's paint `texture` into the page's texture
	/// (all of it on the first paint and after a resize), GPU to GPU, and wait for the
	/// copy: call it from the paint's callback, which must not return before.
	pub fn copy_d3d11(&self, texture: &D3D11Texture, rects: &[Region]) -> Result<(), String> {
		let result = self.import_d3d11_and_copy(texture, rects);
		if result.is_err() {
			self.failed();
		}
		result
	}

	fn import_d3d11_and_copy(&self, texture: &D3D11Texture, rects: &[Region]) -> Result<(), String> {
		if !texture.bgra {
			return Err("the browser painted RGBA; the UI page takes BGRA".into());
		}
		if texture.width == 0 || texture.height == 0 {
			return Err("the browser painted an empty frame".into());
		}
		if texture.handle.is_null() {
			return Err("the paint has no shared texture handle".into());
		}
		let handle = HANDLE(texture.handle);
		let (source, imported) = {
			let mut imports = self.inner.imports.lock().unwrap_or_else(|e| e.into_inner());
			match imports.get(handle, texture.width, texture.height) {
				Some(source) => (source, false),
				None => {
					let started = Instant::now();
					let (own, source) = self.import_d3d11(handle, texture)?;
					imports.keep(own, source.clone());
					self.stats_mut().import += started.elapsed();
					(source, true)
				}
			}
		};
		if imported {
			self.stats_mut().imports += 1;
		}
		// Kept (see `Imports`): read only now, during the paint's callback.
		self.copy_from(&source, rects)
	}

	/// Open the texture behind `handle` on our device, with a handle of our own to it.
	fn import_d3d11(&self, handle: HANDLE, texture: &D3D11Texture) -> Result<(OwnedHandle, wgpu::Texture), String> {
		let own = OwnedHandle::duplicate(handle)?;
		let device = &self.inner.device;
		// SAFETY: the device is Direct3D 12 (`UiPage::new` checked); the handle is ours,
		// to the browser's shared texture; the resource is checked against the paint's
		// description before it is wrapped.
		let hal_texture = unsafe {
			let hal_device = device.as_hal::<wgpu::hal::api::Dx12>().ok_or("the GPU device isn't Direct3D 12")?;
			let mut resource: Option<ID3D12Resource> = None;
			hal_device
				.raw_device()
				.OpenSharedHandle(own.0, &mut resource)
				.map_err(|e| format!("opening the paint's shared texture on Direct3D 12: {e}"))?;
			let resource = resource.ok_or("Direct3D 12 opened no texture for the paint's handle")?;
			let desc = resource.GetDesc();
			if desc.Dimension != D3D12_RESOURCE_DIMENSION_TEXTURE2D || !bgra(desc.Format) {
				return Err(format!(
					"the paint's shared texture isn't a 2D BGRA texture ({:?}, {:?})",
					desc.Dimension, desc.Format
				));
			}
			if (desc.Width, desc.Height) != (u64::from(texture.width), texture.height) {
				return Err(format!(
					"the paint's shared texture is {}x{}, the paint {}x{}",
					desc.Width, desc.Height, texture.width, texture.height
				));
			}
			wgpu::hal::dx12::Device::texture_from_raw(
				resource,
				super::FORMAT,
				wgpu::TextureDimension::D2,
				wgpu::Extent3d { width: texture.width, height: texture.height, depth_or_array_layers: 1 },
				1,
				1,
			)
		};
		let descriptor = wgpu::TextureDescriptor {
			label: Some("browser paint"),
			size: wgpu::Extent3d { width: texture.width, height: texture.height, depth_or_array_layers: 1 },
			mip_level_count: 1,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: super::FORMAT,
			usage: wgpu::TextureUsages::COPY_SRC,
			view_formats: &[],
		};
		// SAFETY: made on this device from the descriptor above; the browser writes it
		// before each paint that hands it over.
		let source = unsafe {
			device.create_texture_from_hal::<wgpu::hal::api::Dx12>(hal_texture, &descriptor, wgpu::TextureUses::UNINITIALIZED)
		};
		Ok((own, source))
	}
}

/// A BGRA format a copy into the page's `Bgra8Unorm` texture takes.
fn bgra(format: DXGI_FORMAT) -> bool {
	[DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_B8G8R8A8_TYPELESS, DXGI_FORMAT_B8G8R8A8_UNORM_SRGB].contains(&format)
}

/// A handle of ours, closed when dropped.
struct OwnedHandle(HANDLE);

// SAFETY: a kernel handle is a process-wide value; any thread may use or close it.
unsafe impl Send for OwnedHandle {}

impl OwnedHandle {
	/// A duplicate of `handle` (to the same object, with the same access).
	fn duplicate(handle: HANDLE) -> Result<OwnedHandle, String> {
		let mut own = HANDLE::default();
		// SAFETY: `handle` is the browser's, open during the paint's callback; the
		// duplicate is ours and closed by `Drop`.
		unsafe {
			let process = GetCurrentProcess();
			DuplicateHandle(process, handle, process, &mut own, 0, false, DUPLICATE_SAME_ACCESS)
				.map_err(|e| format!("duplicating the paint's shared texture handle: {e}"))?;
		}
		Ok(OwnedHandle(own))
	}

	/// Whether `other` is a handle to the same texture.
	fn same_object(&self, other: HANDLE) -> bool {
		// SAFETY: both handles are open (ours until dropped, the browser's during the
		// paint's callback).
		unsafe { CompareObjectHandles(self.0, other) }.as_bool()
	}
}

impl Drop for OwnedHandle {
	fn drop(&mut self) {
		// SAFETY: ours, closed once.
		let _ = unsafe { CloseHandle(self.0) };
	}
}

/// The browser's textures opened so far, each with a handle of ours to it: Chromium
/// paints into a small pool of them. Kept: the [`MAX_IMPORTS`] latest of the current
/// size; a paint of another size (a resize: a new pool) drops the rest.
#[derive(Default)]
pub(super) struct Imports {
	kept: Vec<(OwnedHandle, wgpu::Texture)>,
}

/// More than Chromium's pool (3), so a steady pool is never opened again.
const MAX_IMPORTS: usize = 4;

impl Imports {
	fn get(&mut self, handle: HANDLE, width: u32, height: u32) -> Option<wgpu::Texture> {
		self.kept.retain(|(_, t)| (t.width(), t.height()) == (width, height));
		let at = self.kept.iter().position(|(own, _)| own.same_object(handle))?;
		// The latest used last.
		let entry = self.kept.remove(at);
		let texture = entry.1.clone();
		self.kept.push(entry);
		Some(texture)
	}

	fn keep(&mut self, own: OwnedHandle, texture: wgpu::Texture) {
		if self.kept.len() >= MAX_IMPORTS {
			self.kept.remove(0);
		}
		self.kept.push((own, texture));
	}
}
