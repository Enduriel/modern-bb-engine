//! The game's total CPU and memory: `bb` and every process it started (the page's CEF
//! processes), logged every 5 s on stderr with `BB_STATS=1`.
//!
//! Memory is the proportional set size (PSS): a page several processes share (CEF's
//! library) counts once, split between them, so the sum is what the game costs. CPU is
//! the processes' time over the interval, as a share of one core (100% is a whole core).
//!
//! [`tree_memory`] breaks each process's memory down by what holds it, with the GPU memory
//! it allocated (which PSS doesn't show): the memory inventory's view of the
//! processes.
//!
//! Per operating system: Linux reads `/proc`. [`tree_memory`] also reads Windows'
//! process counters (the processes of the job the game puts itself in, which its children
//! join) and macOS's resource usage (the game and the processes it started), without the
//! breakdown by kind; the usage log is Linux's only, and says so elsewhere.

use std::time::{Duration, Instant};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

/// One of the game's processes and what holds its memory.
#[derive(Clone, Debug)]
pub struct ProcessMemory {
	pub pid: u32,
	/// `game`, or the CEF process's type (`renderer`, `utility:StorageService`, …).
	pub role: String,
	/// Its PSS, by the kind of mapping holding it (see [`MappingKind`]), largest first.
	pub kinds: Vec<(MappingKind, u64)>,
	/// Its RSS by the same kinds, largest first (Linux; empty elsewhere).
	pub rss_kinds: Vec<(MappingKind, u64)>,
	/// GPU memory it allocated: in video memory, and in system memory the GPU maps (GTT).
	pub vram: u64,
	pub gtt: u64,
	/// Its GPU clients (Linux: each DRM client its descriptors hold, once), each with the
	/// memory figures its `fdinfo` gives (`drm-memory-vram`, `drm-shared-vram`,
	/// `drm-total-gtt`, `amd-evicted-vram`, …). On amdgpu the page's GL context (Chromium's
	/// GPU work, in the game's process) shares the game's client, so its memory counts there.
	pub gpu_clients: Vec<GpuClient>,
	/// The system's own figures for it, in bytes, by its names: on Linux `/proc`'s
	/// `status` (`VmRSS`, `VmHWM`, `RssAnon`, `RssFile`, `RssShmem`, `VmSwap`, …), on
	/// Windows the process memory counters (`WorkingSetSize`, `PrivateUsage`, …), on
	/// macOS its resource usage (`phys_footprint`, `resident_size`).
	pub figures: Vec<(String, u64)>,
}

/// One GPU client of a process's (see [`ProcessMemory::gpu_clients`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GpuClient {
	pub id: String,
	/// The driver (`drm-driver`: `amdgpu`, `i915`, …).
	pub driver: String,
	/// Its memory figures in bytes, by their `fdinfo` names.
	pub figures: Vec<(String, u64)>,
}

impl ProcessMemory {
	/// Its PSS in all.
	pub fn pss(&self) -> u64 {
		self.kinds.iter().map(|(_, b)| b).sum()
	}
}

/// What a process's memory mapping holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MappingKind {
	/// The C heap (`brk`): small allocations of the main thread's malloc arena.
	Heap,
	/// Anonymous mappings: large allocations, other threads' malloc arenas, driver buffers.
	Anonymous,
	/// V8's heaps: the page's JavaScript objects and code (`[anon:v8…]` mappings).
	V8,
	/// Chromium's PartitionAlloc: Blink's DOM, layout, styles, decoded images and
	/// strings, and its own malloc (`[anon:partition_alloc]`).
	PartitionAlloc,
	/// CEF's library, its code and the data it relocates at load.
	Libcef,
	/// The GPU driver's libraries and mappings.
	GpuDriver,
	/// Memory shared between processes (CEF's frames and IPC).
	Shared,
	/// Thread stacks.
	Stacks,
	/// Other files: our binary, other libraries, CEF's resource packs.
	Files,
}

impl MappingKind {
	pub fn name(self) -> &'static str {
		match self {
			MappingKind::Heap => "C heap",
			MappingKind::Anonymous => "anonymous",
			MappingKind::V8 => "V8 heap",
			MappingKind::PartitionAlloc => "PartitionAlloc",
			MappingKind::Libcef => "libcef.so",
			MappingKind::GpuDriver => "GPU driver",
			MappingKind::Shared => "shared memory",
			MappingKind::Stacks => "stacks",
			MappingKind::Files => "other files",
		}
	}
}

/// Every process of the game's and what holds its memory, or `None` where it can't be
/// read yet.
pub fn tree_memory() -> Option<Vec<ProcessMemory>> {
	#[cfg(target_os = "linux")]
	{
		Some(linux::tree_memory())
	}
	#[cfg(target_os = "macos")]
	{
		Some(macos::tree_memory())
	}
	#[cfg(windows)]
	{
		windows::tree_memory()
	}
	#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
	{
		None
	}
}

/// What the process `pid` of the game's is: `game`, or the CEF process's type (see
/// [`ProcessMemory::role`]); `None` where it can't be read (off Linux).
pub fn process_role(pid: u32) -> Option<String> {
	#[cfg(target_os = "linux")]
	{
		Some(linux::role(pid))
	}
	#[cfg(not(target_os = "linux"))]
	{
		let _ = pid;
		None
	}
}

/// This process's GPU clients (see [`ProcessMemory::gpu_clients`]); none off Linux.
pub fn own_gpu_clients() -> Vec<GpuClient> {
	#[cfg(target_os = "linux")]
	{
		linux::gpu_clients(std::process::id())
	}
	#[cfg(not(target_os = "linux"))]
	{
		Vec::new()
	}
}

/// The game's processes at one moment.
#[derive(Clone, Copy, Debug)]
pub struct Usage {
	pub processes: usize,
	/// The CPU time the processes alive now have used in all.
	pub cpu: Duration,
	/// Their memory (PSS).
	pub pss_bytes: u64,
}

/// The game's usage now, or `None` where it can't be read yet.
pub fn tree_usage() -> Option<Usage> {
	#[cfg(target_os = "linux")]
	{
		linux::tree_usage()
	}
	#[cfg(not(target_os = "linux"))]
	{
		None
	}
}

/// Log the game's usage every `every` on stderr, from a thread of its own.
pub fn spawn_logger(every: Duration) {
	let spawned = std::thread::Builder::new().name("bb-stats".to_string()).spawn(move || {
		let Some(mut last) = tree_usage() else {
			eprintln!("stats: not available on this operating system yet");
			return;
		};
		let mut since = Instant::now();
		loop {
			std::thread::sleep(every);
			let Some(now) = tree_usage() else { return };
			let wall = since.elapsed().as_secs_f64();
			since = Instant::now();
			// A process that exited takes its time with it: the share then reads low once.
			let cpu = now.cpu.saturating_sub(last.cpu).as_secs_f64() / wall * 100.0;
			eprintln!(
				"stats: {} processes, CPU {cpu:.0}% of a core, memory {} MB (PSS)",
				now.processes,
				now.pss_bytes / (1 << 20)
			);
			last = now;
		}
	});
	if let Err(e) = spawned {
		eprintln!("stats: {e}");
	}
}

