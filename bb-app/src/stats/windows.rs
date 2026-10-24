//! The game's processes on Windows: those of the job the game put itself in
//! (`win_process::job`, which every process it starts joins), each with its memory
//! counters. No breakdown by kind of memory, and no GPU memory (DXGI's budget is the
//! adapter's, not a process's).

use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::JobObjects::{JOBOBJECT_BASIC_PROCESS_ID_LIST, JobObjectBasicProcessIdList, QueryInformationJobObject};
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

use super::ProcessMemory;

/// The most processes read from the job.
const MOST: usize = 256;

pub(super) fn tree_memory() -> Option<Vec<ProcessMemory>> {
	let me = std::process::id();
	let mut pids = job_processes().unwrap_or_default();
	if !pids.contains(&me) {
		pids.insert(0, me);
	}
	Some(pids.into_iter().filter_map(|pid| process_memory(pid, pid == me)).collect())
}

/// The processes in this process's job, if it is in one.
fn job_processes() -> Option<Vec<u32>> {
	// The list's header, then its ids: room for MOST of them.
	let mut buffer = vec![0usize; 2 + MOST];
	let bytes = (buffer.len() * size_of::<usize>()) as u32;
	// SAFETY: no job handle names this process's own job; the buffer is the list's
	// layout (two counts, then the ids as pointer-sized values) and `bytes` long.
	unsafe {
		QueryInformationJobObject(None, JobObjectBasicProcessIdList, buffer.as_mut_ptr().cast(), bytes, None).ok()?;
		let list = &*buffer.as_ptr().cast::<JOBOBJECT_BASIC_PROCESS_ID_LIST>();
		let n = (list.NumberOfProcessIdsInList as usize).min(MOST);
		let ids = std::slice::from_raw_parts(list.ProcessIdList.as_ptr(), n);
		Some(ids.iter().map(|&id| id as u32).collect())
	}
}

fn process_memory(pid: u32, game: bool) -> Option<ProcessMemory> {
	let mut c = PROCESS_MEMORY_COUNTERS_EX::default();
	// SAFETY: the handle is closed below; the counters are the structure of the size
	// passed (the extended one starts with the plain one).
	unsafe {
		let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
		let read = GetProcessMemoryInfo(process, (&raw mut c).cast::<PROCESS_MEMORY_COUNTERS>(), size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32);
		let _ = CloseHandle(process);
		read.ok()?;
	}
	let figures = [
		("WorkingSetSize", c.WorkingSetSize),
		("PeakWorkingSetSize", c.PeakWorkingSetSize),
		("PrivateUsage", c.PrivateUsage),
		("PagefileUsage", c.PagefileUsage),
		("PeakPagefileUsage", c.PeakPagefileUsage),
		("QuotaPagedPoolUsage", c.QuotaPagedPoolUsage),
		("QuotaNonPagedPoolUsage", c.QuotaNonPagedPoolUsage),
	];
	Some(ProcessMemory {
		pid,
		role: if game { "game" } else { "child" }.to_string(),
		kinds: Vec::new(),
		rss_kinds: Vec::new(),
		vram: 0,
		gtt: 0,
		gpu_clients: Vec::new(),
		figures: figures.into_iter().map(|(n, b)| (n.to_string(), b as u64)).collect(),
	})
}
