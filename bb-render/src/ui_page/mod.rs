//! [`UiPage`]: the UI page's latest frame on the GPU.
//!
//! Chromium paints the page on the GPU and hands each paint over as one of its own GPU
//! buffers (CEF's accelerated paint, a shared texture): a DMA-BUF on Linux ([`linux`]),
//! an IOSurface on macOS ([`macos`]) and a D3D11 shared texture on Windows
//! ([`windows`]).
//! Each paint is imported and the parts it changed (its dirty rectangles) are copied,
//! GPU to GPU, into a texture of ours, which [`UiLayer`](crate::UiLayer) blends over
//! the game's frame. Nothing of the page passes through the CPU.
//!
//! CEF's rules for the buffer: it may only be used during the paint's callback, comes
//! from a pool (a different buffer each paint), and goes back to the pool when the
//! callback returns. So the copy is submitted and waited for (that submission only, not
//! the whole device) before [`UiPage::copy_dmabuf`] (or `copy_iosurface`, `copy_d3d11`)
//! returns. Each pooled buffer is imported once and kept while the browser keeps handing
//! it over (`imports`; on macOS each pooled IOSurface likewise, see `macos`, and on
//! Windows each pooled texture, see `windows`): the import holds
//! its own reference to the buffer and is only read during a callback that hands over
//! that buffer. It runs on CEF's thread (on macOS the main thread, which runs CEF's work
//! for the game); the device and queue are
//! shared with the game's thread, whose submissions it is ordered with by the queue.
//! It doesn't submit while the game's thread configures the window's surface, which
//! another thread's submission would fail ([`Gpu::configure_surface`]).
//!
//! The pixels themselves are read back only on demand ([`UiPage::read`]): a small
//! region for "is the cursor over the UI" (bb-ui caches it per paint), the whole page
//! for a screenshot.

#[cfg(target_os = "linux")]
mod imports;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

#[cfg(target_os = "linux")]
pub use linux::DmaBuf;
#[cfg(target_os = "macos")]
pub use macos::IOSurface;
#[cfg(windows)]
pub use windows::D3D11Texture;

use crate::gpu::Gpu;

/// The page's texture format: Chromium's paints are BGRA, premultiplied.
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8Unorm;

/// How long a copy or a read may wait on the GPU before it counts as failed (a hung
/// GPU mustn't hang CEF's thread for good).
const GPU_WAIT: Duration = Duration::from_secs(2);

/// A part of the page's frame, in its pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
	pub x: u32,
	pub y: u32,
	pub width: u32,
	pub height: u32,
}

impl Region {
	/// This region clipped to a `width` × `height` frame; `None` if nothing of it is in it.
	fn clipped(&self, width: u32, height: u32) -> Option<Region> {
		let (x, y) = (self.x.min(width), self.y.min(height));
		let (w, h) = (self.width.min(width - x), self.height.min(height - y));
		(w > 0 && h > 0).then_some(Region { x, y, width: w, height: h })
	}
}

/// What copying the page's paints has cost so far.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CopyStats {
	/// Paints copied.
	pub paints: u64,
	/// Rectangles copied (a paint's dirty rectangles).
	pub rects: u64,
	/// Pixels copied.
	pub pixels: u64,
	/// The paints' buffers imported (a buffer handed over again isn't), and the time
	/// that took.
	pub imports: u64,
	pub import: Duration,
	/// Time recording, submitting and waiting for the copies.
	pub copy: Duration,
	/// Paints that couldn't be imported or copied.
	pub failures: u64,
	/// Reads back to the CPU, and the time they took.
	pub reads: u64,
	pub read_time: Duration,
}

impl CopyStats {
	/// What happened between `earlier` and these.
	pub fn since(&self, earlier: &CopyStats) -> CopyStats {
		CopyStats {
			paints: self.paints - earlier.paints,
			rects: self.rects - earlier.rects,
			pixels: self.pixels - earlier.pixels,
			imports: self.imports - earlier.imports,
			import: self.import.saturating_sub(earlier.import),
			copy: self.copy.saturating_sub(earlier.copy),
			failures: self.failures - earlier.failures,
			reads: self.reads - earlier.reads,
			read_time: self.read_time.saturating_sub(earlier.read_time),
		}
	}
}

/// The page's texture: replaced when the page's size changes.
pub(crate) struct PageTexture {
	pub texture: wgpu::Texture,
	pub view: wgpu::TextureView,
	/// Counts up with every new texture, so a bind group made for it knows it's stale.
	pub generation: u64,
}

struct Inner {
	device: wgpu::Device,
	queue: wgpu::Queue,
	/// Held while submitting (see [`Gpu::configure_surface`]).
	submits: Arc<Mutex<()>>,
	page: Mutex<Option<PageTexture>>,
	generations: AtomicU64,
	stats: Mutex<CopyStats>,
	/// The browser's paint buffers imported so far (kept while it reuses them).
	#[cfg(target_os = "linux")]
	imports: Mutex<imports::Imports>,
	/// macOS: the browser's pooled surfaces, imported once each (see `macos`).
	#[cfg(target_os = "macos")]
	imports: Mutex<macos::Imports>,
	/// Windows: the browser's pooled textures, opened once each (see `windows`).
	#[cfg(windows)]
	imports: Mutex<windows::Imports>,
}

/// The page's latest frame on the GPU, shared between the browser's paint thread
/// (which copies paints into it) and the game's (which draws it, [`UiLayer`](crate::UiLayer)).
/// Cloning shares it.
#[derive(Clone)]
pub struct UiPage {
	inner: Arc<Inner>,
}

impl UiPage {
	/// The device features importing the browser's paints needs (Linux: Vulkan's
	/// DMA-BUF import; macOS and Windows: none, Metal and Direct3D 12 do it), for [`Gpu::headless_with`] and
	/// [`Gpu::for_surface_with`].
	pub const FEATURES: wgpu::Features = features();

	/// The page on `gpu`, which must have been opened with [`UiPage::FEATURES`]: fails,
	/// saying why, if it can't take the browser's paints.
	pub fn new(gpu: &Gpu) -> Result<UiPage, String> {
		if !gpu.device.features().contains(Self::FEATURES) {
			return Err(format!(
				"the GPU device ({}) wasn't opened with the features the UI page needs ({:?})",
				gpu.adapter,
				Self::FEATURES
			));
		}
		supported(gpu)?;
		Ok(UiPage {
			inner: Arc::new(Inner {
				device: gpu.device.clone(),
				queue: gpu.queue.clone(),
				submits: gpu.submits_gate(),
				page: Mutex::new(None),
				generations: AtomicU64::new(0),
				stats: Mutex::new(CopyStats::default()),
				#[cfg(target_os = "linux")]
				imports: Mutex::new(imports::Imports::default()),
				#[cfg(target_os = "macos")]
				imports: Mutex::new(macos::Imports::default()),
				#[cfg(windows)]
				imports: Mutex::new(windows::Imports::default()),
			}),
		})
	}

	fn page(&self) -> MutexGuard<'_, Option<PageTexture>> {
		self.inner.page.lock().unwrap_or_else(|e| e.into_inner())
	}

	fn stats_mut(&self) -> MutexGuard<'_, CopyStats> {
		self.inner.stats.lock().unwrap_or_else(|e| e.into_inner())
	}

	/// The page's texture, made anew at `width` × `height` if it's another size (or
	/// there is none yet); whether it's new (its contents undefined).
	fn texture_for<'a>(&self, page: &'a mut Option<PageTexture>, width: u32, height: u32) -> (&'a PageTexture, bool) {
		let fits = matches!(page, Some(p) if (p.texture.width(), p.texture.height()) == (width, height));
		if !fits {
			let texture = self.inner.device.create_texture(&wgpu::TextureDescriptor {
				label: Some("ui page"),
				size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
				mip_level_count: 1,
				sample_count: 1,
				dimension: wgpu::TextureDimension::D2,
				format: FORMAT,
				usage: wgpu::TextureUsages::TEXTURE_BINDING
					| wgpu::TextureUsages::COPY_DST
					| wgpu::TextureUsages::COPY_SRC,
				view_formats: &[],
			});
			let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
			let generation = self.inner.generations.fetch_add(1, Ordering::Relaxed) + 1;
			*page = Some(PageTexture { texture, view, generation });
		}
		(page.as_ref().expect("made above"), !fits)
	}

	/// Copy `rects` of `source` (a `width` × `height` paint) into the page's texture,
	/// all of it if the texture is new or another size; submit the copy and wait for it.
	fn copy_from(&self, source: &wgpu::Texture, rects: &[Region]) -> Result<(), String> {
		let (width, height) = (source.width(), source.height());
		let started = Instant::now();
		let mut copied = (0u64, 0u64);
		let submitted = {
			let mut page = self.page();
			let (page, new) = self.texture_for(&mut page, width, height);
			let whole = [Region { x: 0, y: 0, width, height }];
			let rects = if new { &whole[..] } else { rects };
			let mut encoder = self
				.inner
				.device
				.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("ui page copy") });
			for rect in rects.iter().filter_map(|r| r.clipped(width, height)) {
				let origin = wgpu::Origin3d { x: rect.x, y: rect.y, z: 0 };
				encoder.copy_texture_to_texture(
					wgpu::TexelCopyTextureInfo { texture: source, mip_level: 0, origin, aspect: wgpu::TextureAspect::All },
					wgpu::TexelCopyTextureInfo { texture: &page.texture, mip_level: 0, origin, aspect: wgpu::TextureAspect::All },
					wgpu::Extent3d { width: rect.width, height: rect.height, depth_or_array_layers: 1 },
				);
				copied.0 += 1;
				copied.1 += u64::from(rect.width) * u64::from(rect.height);
			}
			self.submit(encoder)
		};
		// The source goes back to the browser's pool when the paint's callback returns.
		self.wait(submitted)?;
		let mut stats = self.stats_mut();
		stats.paints += 1;
		stats.rects += copied.0;
		stats.pixels += copied.1;
		stats.copy += started.elapsed();
		Ok(())
	}

	/// Submit `encoder`'s commands, not while the window's surface is configured (see
	/// [`Gpu::configure_surface`]): this runs on CEF's thread.
	fn submit(&self, encoder: wgpu::CommandEncoder) -> wgpu::SubmissionIndex {
		let _gate = crate::gpu::lock(&self.inner.submits);
		self.inner.queue.submit([encoder.finish()])
	}

	fn wait(&self, submission: wgpu::SubmissionIndex) -> Result<(), String> {
		self.inner
			.device
			.poll(wgpu::PollType::Wait { submission_index: Some(submission), timeout: Some(GPU_WAIT) })
			.map(|_| ())
			.map_err(|e| format!("waiting for the GPU: {e}"))
	}

	// Only the platforms with a paint import count failures.
	#[cfg(any(target_os = "linux", target_os = "macos", windows))]
	fn failed(&self) {
		self.stats_mut().failures += 1;
	}

	/// The page's pixels in `region` (clipped to the page), read back from the GPU: BGRA,
	/// premultiplied, rows top to bottom, with the region's size as clipped; `None` before
	/// the first paint or if nothing of the region is on the page.
	pub fn read(&self, region: Region) -> Result<Option<(Region, Vec<u8>)>, String> {
		let started = Instant::now();
		let (buffer, region, stride, submitted) = {
			let page = self.page();
			let Some(page) = page.as_ref() else { return Ok(None) };
			let Some(region) = region.clipped(page.texture.width(), page.texture.height()) else {
				return Ok(None);
			};
			let stride = (region.width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
			let buffer = self.inner.device.create_buffer(&wgpu::BufferDescriptor {
				label: Some("ui page read"),
				size: u64::from(stride) * u64::from(region.height),
				usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
				mapped_at_creation: false,
			});
			let mut encoder = self
				.inner
				.device
				.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("ui page read") });
			encoder.copy_texture_to_buffer(
				wgpu::TexelCopyTextureInfo {
					texture: &page.texture,
					mip_level: 0,
					origin: wgpu::Origin3d { x: region.x, y: region.y, z: 0 },
					aspect: wgpu::TextureAspect::All,
				},
				wgpu::TexelCopyBufferInfo {
					buffer: &buffer,
					layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(stride), rows_per_image: None },
				},
				wgpu::Extent3d { width: region.width, height: region.height, depth_or_array_layers: 1 },
			);
			let submitted = self.submit(encoder);
			(buffer, region, stride, submitted)
		};
		buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
		self.wait(submitted)?;
		let mut pixels = Vec::with_capacity((region.width * region.height * 4) as usize);
		{
			let data = buffer.slice(..).get_mapped_range().map_err(|e| format!("reading the UI page: {e:?}"))?;
			let row = region.width as usize * 4;
			for y in 0..region.height as usize {
				pixels.extend_from_slice(&data[y * stride as usize..][..row]);
			}
		}
		let mut stats = self.stats_mut();
		stats.reads += 1;
		stats.read_time += started.elapsed();
		Ok(Some((region, pixels)))
	}

	/// Chromium switches that the browser's paints need to reach this page, as
	/// `(name, value)`: on Windows, wgpu's adapter for Chromium's GPU work (see
	/// `windows`); none elsewhere.
	pub fn browser_switches(&self) -> Vec<(&'static str, String)> {
		#[cfg(windows)]
		{
			windows::adapter_switch(&self.inner.device).into_iter().collect()
		}
		#[cfg(not(windows))]
		{
			Vec::new()
		}
	}

	/// The page's size, once painted.
	pub fn size(&self) -> Option<(u32, u32)> {
		self.page().as_ref().map(|p| (p.texture.width(), p.texture.height()))
	}

	/// The bytes the page's texture takes on the GPU (none before the first paint).
	pub fn bytes(&self) -> u64 {
		self.page().as_ref().map_or(0, |p| crate::texture::texture_bytes(&p.texture))
	}

	/// What copying the paints (and reading them back) has cost so far.
	pub fn copy_stats(&self) -> CopyStats {
		*self.stats_mut()
	}

	/// The current texture's view and generation, for drawing it.
	pub(crate) fn current(&self) -> Option<(wgpu::TextureView, u64)> {
		self.page().as_ref().map(|p| (p.view.clone(), p.generation))
	}

	/// Tests only (the browser paints the page in the game): make the page
	/// `width` × `height` pixels of `bgra` (premultiplied, rows top to bottom), with
	/// only `rects` of it written if the page has that size already.
	#[doc(hidden)]
	pub fn write_for_tests(&self, width: u32, height: u32, bgra: &[u8], rects: &[Region]) {
		let source = self.inner.device.create_texture(&wgpu::TextureDescriptor {
			label: Some("ui page test paint"),
			size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
			mip_level_count: 1,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: FORMAT,
			usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
			view_formats: &[],
		});
		self.inner.queue.write_texture(
			source.as_image_copy(),
			bgra,
			wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(width * 4), rows_per_image: None },
			source.size(),
		);
		self.copy_from(&source, rects).expect("the test paint copies");
	}
}

/// See [`UiPage::FEATURES`].
const fn features() -> wgpu::Features {
	#[cfg(target_os = "linux")]
	{
		wgpu::Features::VULKAN_EXTERNAL_MEMORY_DMA_BUF
	}
	#[cfg(not(target_os = "linux"))]
	{
		wgpu::Features::empty()
	}
}

/// Whether this platform's paints can be taken on `gpu`.
fn supported(gpu: &Gpu) -> Result<(), String> {
	#[cfg(target_os = "linux")]
	{
		linux::supported(gpu)
	}
	#[cfg(target_os = "macos")]
	{
		macos::supported(gpu)
	}
	#[cfg(windows)]
	{
		windows::supported(gpu)
	}
	#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
	{
		let _ = gpu;
		Err("the UI page's shared GPU textures aren't implemented on this operating system yet".into())
	}
}
