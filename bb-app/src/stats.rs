//! The game's total CPU and memory: `bb` and every process it started (the page's CEF
//! processes), logged every 5 s on stderr with `BB_STATS=1`.
//!
//! Memory is the proportional set size (PSS): a page several processes share (CEF's
//! library) counts once, split between them, so the sum is what the game costs. CPU is
//! the processes' time over the interval, as a share of one core (100% is a whole core).
//!
//! Per operating system: Linux reads `/proc`. Windows (a job object the game puts
//! itself in, which its children join: its accounting, and kill-on-close) and macOS come
//! later; until then the log says it isn't available.

use std::time::{Duration, Instant};

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

#[cfg(target_os = "linux")]
mod linux {
	use std::time::Duration;

	use super::Usage;

	/// `/proc`'s times are in clock ticks of `USER_HZ`, 100 on every Linux.
	const TICKS_PER_SECOND: u64 = 100;

	pub(super) fn tree_usage() -> Option<Usage> {
		let mut todo = vec![std::process::id()];
		let mut pids = Vec::new();
		while let Some(pid) = todo.pop() {
			pids.push(pid);
			todo.extend(children(pid));
		}
		let ticks: u64 = pids.iter().filter_map(|&p| cpu_ticks(p)).sum();
		let pss_kb: u64 = pids.iter().filter_map(|&p| pss_kb(p)).sum();
		Some(Usage {
			processes: pids.len(),
			cpu: Duration::from_millis(ticks * 1000 / TICKS_PER_SECOND),
			pss_bytes: pss_kb * 1024,
		})
	}

	/// The processes `pid` started (from any of its threads).
	fn children(pid: u32) -> Vec<u32> {
		let Ok(tasks) = std::fs::read_dir(format!("/proc/{pid}/task")) else { return Vec::new() };
		tasks
			.flatten()
			.filter_map(|t| std::fs::read_to_string(t.path().join("children")).ok())
			.flat_map(|list| list.split_whitespace().filter_map(|p| p.parse().ok()).collect::<Vec<u32>>())
			.collect()
	}

	/// User and system time, in ticks (`stat`'s 14th and 15th fields, after the name).
	fn cpu_ticks(pid: u32) -> Option<u64> {
		let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
		let fields: Vec<&str> = stat.rsplit_once(')')?.1.split_whitespace().collect();
		Some(fields.get(11)?.parse::<u64>().ok()? + fields.get(12)?.parse::<u64>().ok()?)
	}

	/// The process's proportional set size, in kB.
	fn pss_kb(pid: u32) -> Option<u64> {
		let rollup = std::fs::read_to_string(format!("/proc/{pid}/smaps_rollup")).ok()?;
		rollup.lines().find_map(|l| l.strip_prefix("Pss:")?.split_whitespace().next()?.parse().ok())
	}

	#[cfg(test)]
	mod tests {
		#[test]
		fn reads_this_process() {
			let usage = super::tree_usage().expect("usage");
			assert!(usage.processes >= 1);
			assert!(usage.pss_bytes > 0, "a process has memory");
		}
	}
}
