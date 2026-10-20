//! Each submission's GPU time, by name, for profiling the frame on the GPU (`bb`'s
//! `BB_GPU_TIMES=1` with `BB_FRAME_TIMES`). Off unless the device was opened with it
//! (see [`wanted`]); then each named submission (see [`crate::Gpu::submit`]) is wrapped
//! in two timestamps, and [`PassTimer::take`] waits for the GPU and reads them.

use std::cell::RefCell;
use std::sync::mpsc::channel;
use std::time::Duration;

/// The submissions timed in one frame at most; more go untimed.
const MOST: u32 = 64;

/// The features the timer needs: timestamps written between commands.
pub(crate) const FEATURES: wgpu::Features =
	wgpu::Features::TIMESTAMP_QUERY.union(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS);

/// Whether the environment asks for the passes' GPU times (`BB_GPU_TIMES=1`).
pub(crate) fn wanted() -> bool {
	std::env::var_os("BB_GPU_TIMES").is_some_and(|v| !v.is_empty() && v != "0")
}

/// Timestamps around named submissions.
pub(crate) struct PassTimer {
	set: wgpu::QuerySet,
	resolve: wgpu::Buffer,
	read: wgpu::Buffer,
	/// The names of the submissions timed since the last take, a pair of queries each.
	names: RefCell<Vec<&'static str>>,
	/// Nanoseconds a timestamp tick.
	period: f64,
}

impl PassTimer {
	pub(crate) fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> PassTimer {
		let size = u64::from(MOST) * 2 * 8;
		PassTimer {
			set: device.create_query_set(&wgpu::QuerySetDescriptor {
				label: Some("pass times"),
				ty: wgpu::QueryType::Timestamp,
				count: MOST * 2,
			}),
			resolve: device.create_buffer(&wgpu::BufferDescriptor {
				label: Some("pass times resolved"),
				size,
				usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
				mapped_at_creation: false,
			}),
			read: device.create_buffer(&wgpu::BufferDescriptor {
				label: Some("pass times read"),
				size,
				usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
				mapped_at_creation: false,
			}),
			names: RefCell::new(Vec::new()),
			period: f64::from(queue.get_timestamp_period()),
		}
	}

	/// Submit `commands` between two timestamps, as the submission `name`.
	pub(crate) fn submit(&self, device: &wgpu::Device, queue: &wgpu::Queue, name: &'static str, commands: wgpu::CommandBuffer) {
		let mut names = self.names.borrow_mut();
		if names.len() as u32 >= MOST {
			queue.submit([commands]);
			return;
		}
		let first = names.len() as u32 * 2;
		names.push(name);
		let stamp = |index: u32| {
			let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("pass time") });
			encoder.write_timestamp(&self.set, index);
			encoder.finish()
		};
		queue.submit([stamp(first), commands, stamp(first + 1)]);
	}

	/// Wait for the GPU, and give `each` the time of every submission timed since the
	/// last take, by name, in order.
	pub(crate) fn take(&self, device: &wgpu::Device, queue: &wgpu::Queue, mut each: impl FnMut(&'static str, Duration)) {
		let names = std::mem::take(&mut *self.names.borrow_mut());
		if names.is_empty() {
			return;
		}
		let queries = names.len() as u32 * 2;
		let bytes = u64::from(queries) * 8;
		let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("pass times") });
		encoder.resolve_query_set(&self.set, 0..queries, &self.resolve, 0);
		encoder.copy_buffer_to_buffer(&self.resolve, 0, &self.read, 0, bytes);
		queue.submit([encoder.finish()]);
		let (sender, receiver) = channel();
		self.read.slice(..bytes).map_async(wgpu::MapMode::Read, move |result| {
			let _ = sender.send(result);
		});
		let _ = device.poll(wgpu::PollType::wait_indefinitely());
		if !matches!(receiver.recv(), Ok(Ok(()))) {
			return;
		}
		{
			let Ok(data) = self.read.slice(..bytes).get_mapped_range() else { return };
			let ticks: Vec<u64> = data.chunks_exact(8).map(|b| u64::from_le_bytes(b.try_into().expect("8 bytes"))).collect();
			for (i, name) in names.iter().enumerate() {
				let (begin, end) = (ticks[i * 2], ticks[i * 2 + 1]);
				let nanos = end.saturating_sub(begin) as f64 * self.period;
				each(name, Duration::from_nanos(nanos as u64));
			}
		}
		self.read.unmap();
	}
}
