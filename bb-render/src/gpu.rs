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
			..Default::default()
		};
		let (device, queue) = block_on(adapter.request_device(&descriptor))
			.map_err(|e| format!("no GPU device: {e}"))?;
		Ok(Gpu {
			device,
			queue,
			adapter: format!("{} ({:?})", info.name, info.backend),
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
			..Default::default()
		};
		let (device, queue) = block_on(adapter.request_device(&descriptor))
			.map_err(|e| format!("no GPU device: {e}"))?;
		Ok(Gpu {
			device,
			queue,
			adapter: format!("{} ({:?})", info.name, info.backend),
			surface_adapter: Some(adapter),
		})
	}

	/// The formats, present modes and alpha modes `surface` supports on this device's
	/// adapter (`None` for a headless device).
	pub fn surface_capabilities(&self, surface: &wgpu::Surface) -> Option<wgpu::SurfaceCapabilities> {
		self.surface_adapter.as_ref().map(|a| surface.get_capabilities(a))
	}

	/// Block until the GPU has finished all submitted work.
	pub fn wait(&self) -> Result<(), String> {
		self.device
			.poll(wgpu::PollType::wait_indefinitely())
			.map(|_| ())
			.map_err(|e| e.to_string())
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
