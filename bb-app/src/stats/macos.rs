//! The game's processes on macOS: the game and every process it started (the page's
//! helpers), each with its resource usage (`proc_pid_rusage`): its footprint (what
//! Activity Monitor shows as Memory) and resident size. A process may read its own
//! children's without privileges. No breakdown by kind of memory.

use std::ffi::{c_int, c_void};

use super::ProcessMemory;

/// `RUSAGE_INFO_V2` (`<sys/resource.h>`).
const RUSAGE_INFO_V2: c_int = 2;

/// `struct rusage_info_v2` (`<sys/resource.h>`).
#[repr(C)]
#[derive(Default)]
struct RusageInfoV2 {
	uuid: [u8; 16],
	user_time: u64,
	system_time: u64,
	pkg_idle_wkups: u64,
	interrupt_wkups: u64,
	pageins: u64,
	wired_size: u64,
	resident_size: u64,
	phys_footprint: u64,
	proc_start_abstime: u64,
	proc_exit_abstime: u64,
	child_user_time: u64,
	child_system_time: u64,
	child_pkg_idle_wkups: u64,
	child_interrupt_wkups: u64,
	child_pageins: u64,
	child_elapsed_abstime: u64,
	diskio_bytesread: u64,
	diskio_byteswritten: u64,
}

// libproc, in libSystem.
unsafe extern "C" {
	fn proc_pid_rusage(pid: c_int, flavor: c_int, buffer: *mut c_void) -> c_int;
	fn proc_listchildpids(ppid: c_int, buffer: *mut c_void, buffersize: c_int) -> c_int;
}

pub(super) fn tree_memory() -> Vec<ProcessMemory> {
	let me = std::process::id() as c_int;
	let mut todo = vec![me];
	let mut out = Vec::new();
	while let Some(pid) = todo.pop() {
		todo.extend(children(pid));
		out.extend(process_memory(pid, pid == me));
	}
	out
}

/// The processes `pid` started.
fn children(pid: c_int) -> Vec<c_int> {
	let mut pids = vec![0 as c_int; 256];
	let bytes = (pids.len() * size_of::<c_int>()) as c_int;
	// SAFETY: the buffer holds `bytes` bytes of pids; the call returns how many it wrote.
	let n = unsafe { proc_listchildpids(pid, pids.as_mut_ptr().cast(), bytes) };
	pids.truncate(n.clamp(0, 256) as usize);
	pids.retain(|&p| p > 0);
	pids
}

fn process_memory(pid: c_int, game: bool) -> Option<ProcessMemory> {
	let mut info = RusageInfoV2::default();
	// SAFETY: the buffer is the version-2 structure the flavor names.
	if unsafe { proc_pid_rusage(pid, RUSAGE_INFO_V2, (&raw mut info).cast()) } != 0 {
		return None;
	}
	let figures = [
		("phys_footprint", info.phys_footprint),
		("resident_size", info.resident_size),
		("wired_size", info.wired_size),
	];
	Some(ProcessMemory {
		pid: pid as u32,
		role: if game { "game" } else { "child" }.to_string(),
		kinds: Vec::new(),
		rss_kinds: Vec::new(),
		vram: 0,
		gtt: 0,
		figures: figures.into_iter().map(|(n, b)| (n.to_string(), b)).collect(),
	})
}
