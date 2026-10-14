//! [`Gpu`]: the wgpu device and queue, opened without a window or for one.
//!
//! Frames render into offscreen targets: tests read them back as images, and a window
//! presents them on its surface ([`Presenter`](crate::Presenter)).

use std::future::Future;
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

/// The device and queue every renderer object is created on.
pub struct Gpu {
	pub device: wgpu::Device,
	pub queue: wgpu::Queue,
	/// The adapter's name and backend, for logs.
	pub adapter: String,
	/// Whether the GPU has memory of its own, shares the system's, or is software.
	pub device_type: wgpu::DeviceType,
	/// The adapter, kept when opened for a window (to query its surface).
	surface_adapter: Option<wgpu::Adapter>,
}

impl Gpu {
	/// Open the best available adapter with no surface. Honours wgpu's environment
	/// variables (e.g. `WGPU_BACKEND=vulkan`, `WGPU_ADAPTER_NAME=llvmpipe`).
	pub fn headless() -> Result<Gpu, String> {
		let instance =
			wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
		let options = wgpu::RequestAdapterOptions {
			power_preference: wgpu::PowerPreference::from_env()
				.unwrap_or(wgpu::PowerPreference::HighPerformance),
			..Default::default()
		};
		let adapter = block_on(instance.request_adapter(&options))
			.map_err(|e| format!("no GPU adapter: {e}"))?;
		let info = adapter.get_info();
		let descriptor = wgpu::DeviceDescriptor {
			label: Some("bb-render"),
			memory_hints: memory_hints(),
			..Default::default()
		};
		let (device, queue) = block_on(adapter.request_device(&descriptor))
			.map_err(|e| format!("no GPU device: {e}"))?;
		Ok(Gpu {
			device,
			queue,
			adapter: format!("{} ({:?})", info.name, info.backend),
			device_type: info.device_type,
			surface_adapter: None,
		})
	}

	/// Open an adapter that can present to `surface` (made from `instance`).
	pub fn for_surface(instance: &wgpu::Instance, surface: &wgpu::Surface) -> Result<Gpu, String> {
		let options = wgpu::RequestAdapterOptions {
			power_preference: wgpu::PowerPreference::from_env()
				.unwrap_or(wgpu::PowerPreference::HighPerformance),
			compatible_surface: Some(surface),
			..Default::default()
		};
		let adapter = block_on(instance.request_adapter(&options))
			.map_err(|e| format!("no GPU adapter for the window: {e}"))?;
		let info = adapter.get_info();
		let descriptor = wgpu::DeviceDescriptor {
			label: Some("bb-render"),
			memory_hints: memory_hints(),
			..Default::default()
		};
		let (device, queue) = block_on(adapter.request_device(&descriptor))
			.map_err(|e| format!("no GPU device: {e}"))?;
		Ok(Gpu {
			device,
			queue,
			adapter: format!("{} ({:?})", info.name, info.backend),
			device_type: info.device_type,
			surface_adapter: Some(adapter),
		})
	}

	/// The formats, present modes and alpha modes `surface` supports on this device's
	/// adapter (`None` for a headless device).
	pub fn surface_capabilities(&self, surface: &wgpu::Surface) -> Option<wgpu::SurfaceCapabilities> {
		self.surface_adapter.as_ref().map(|a| surface.get_capabilities(a))
	}

	/// The GPU memory wgpu's allocator holds: the bytes allocated, and the bytes its
	/// memory blocks reserve (free space inside them included), if the backend says.
	pub fn allocator_use(&self) -> Option<(u64, u64)> {
		let report = self.device.generate_allocator_report()?;
		Some((report.total_allocated_bytes, report.total_reserved_bytes))
	}

	/// The textures kept before the least recently drawn go (see `crate::residency`):
	/// 1 GiB on a GPU with memory of its own, 384 MiB on one that shares the system's
	/// (integrated chips, the Steam Deck's) or a software one, where every byte comes
	/// out of the game's RAM. A fresh world draws about 260 MiB of whole atlases, and a
	/// battle adds 100–150 more, so a discrete GPU keeps both and a shared one keeps the
	/// scene in view. The settings file can set it (`render.texture_budget_mib`).
	pub fn texture_budget(&self) -> u64 {
		match self.device_type {
			wgpu::DeviceType::DiscreteGpu => 1 << 30,
			_ => 384 << 20,
		}
	}

	/// Block until the GPU has finished all submitted work.
	pub fn wait(&self) -> Result<(), String> {
		self.device
			.poll(wgpu::PollType::wait_indefinitely())
			.map(|_| ())
			.map_err(|e| e.to_string())
	}
}

/// The largest memory block wgpu's allocator shares between resources: anything larger
/// (every atlas from 2048×1024 up) gets a memory allocation of its own, of its exact
/// size. The default (`MemoryHints::Performance`) shares blocks of 128–256 MiB, and a
/// 42.8 MiB atlas then sits alone in a 64 MiB block: on a fresh world (measured
/// on RADV) its blocks reserved 768 MiB for 481 MiB of resources; with 8 MiB
/// blocks, 488 MiB, at the same frame cost (about 1.8 ms a frame either way).
const SHARED_BLOCK: u64 = 8 << 20;

/// The allocator's block sizes (see [`SHARED_BLOCK`]).
fn memory_hints() -> wgpu::MemoryHints {
	wgpu::MemoryHints::Manual {
		suballocated_device_memory_block_size: SHARED_BLOCK..SHARED_BLOCK,
	}
}

/// Run a future to completion on this thread. wgpu's native futures are ready
/// immediately or once the device is polled, so no executor is needed.
pub(crate) fn block_on<F: Future>(future: F) -> F::Output {
	struct Unpark(std::thread::Thread);
	impl Wake for Unpark {
		fn wake(self: Arc<Self>) {
			self.0.unpark();
		}
	}
	let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
	let mut context = Context::from_waker(&waker);
	let mut future = pin!(future);
	loop {
		match future.as_mut().poll(&mut context) {
			Poll::Ready(output) => return output,
			Poll::Pending => std::thread::park(),
		}
	}
}
