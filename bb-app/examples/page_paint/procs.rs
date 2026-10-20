//! What the game's processes did between two samples, from `/proc` (Linux): CPU time
//! by thread (`schedstat`, in nanoseconds), wake-ups (voluntary context switches: each
//! is a thread going to sleep, so waking again), and the GPU's memory and busy time
//! per process (the DRM clients' `fdinfo`, amdgpu and others that report them).

use std::collections::{HashMap, HashSet};
use std::time::Duration;

/// One sample of every thread of `procs` (pid, role).
#[derive(Default)]
pub struct Sample {
	/// By "role: thread name": CPU time and voluntary context switches.
	threads: HashMap<String, (Duration, u64)>,
	/// By role and DRM client (a GPU driver instance: the game's wgpu, Chromium's GL):
	/// GPU busy time (all engines), and memory (VRAM + GTT, bytes).
	gpu: HashMap<String, (Duration, u64)>,
}

fn read(path: &str) -> Option<String> {
	std::fs::read_to_string(path).ok()
}

fn thread(task: &str) -> Option<(String, Duration, u64)> {
	let name = read(&format!("{task}/comm"))?.trim().to_string();
	let run_ns = read(&format!("{task}/schedstat"))?.split_whitespace().next()?.parse::<u64>().ok()?;
	let status = read(&format!("{task}/status"))?;
	let switches = status
		.lines()
		.find_map(|l| l.strip_prefix("voluntary_ctxt_switches:"))
		.and_then(|v| v.trim().parse().ok())
		.unwrap_or(0);
	Some((name, Duration::from_nanos(run_ns), switches))
}

/// The GPU's busy time and memory of each of `pid`'s DRM clients, by client id.
fn gpu(pid: u32) -> Vec<(String, Duration, u64)> {
	let mut seen = HashSet::new();
	let mut clients = Vec::new();
	let Ok(fds) = std::fs::read_dir(format!("/proc/{pid}/fdinfo")) else { return clients };
	for fd in fds.flatten() {
		let Ok(info) = std::fs::read_to_string(fd.path()) else { continue };
		let Some(client) = info.lines().find_map(|l| l.strip_prefix("drm-client-id:")) else { continue };
		if !seen.insert(client.trim().to_string()) {
			continue;
		}
		let (mut busy, mut bytes) = (0u64, 0u64);
		for line in info.lines() {
			let Some((key, value)) = line.split_once(':') else { continue };
			let mut words = value.split_whitespace();
			let number = words.next().and_then(|n| n.parse::<u64>().ok()).unwrap_or(0);
			if key.starts_with("drm-engine-") && !key.starts_with("drm-engine-capacity") {
				busy += number;
			} else if key == "drm-total-vram" || key == "drm-total-gtt" {
				bytes += number * if words.next() == Some("KiB") { 1024 } else { 1 };
			}
		}
		clients.push((client.trim().to_string(), Duration::from_nanos(busy), bytes));
	}
	clients
}

impl Sample {
	pub fn take(procs: &[(u32, String)]) -> Sample {
		let mut sample = Sample::default();
		for (pid, role) in procs {
			if let Ok(tasks) = std::fs::read_dir(format!("/proc/{pid}/task")) {
				for task in tasks.flatten() {
					if let Some((name, cpu, switches)) = thread(&task.path().to_string_lossy()) {
						let entry = sample.threads.entry(format!("{role}: {name}")).or_default();
						entry.0 += cpu;
						entry.1 += switches;
					}
				}
			}
			for (client, busy, bytes) in gpu(*pid) {
				sample.gpu.insert(format!("{role} #{client}"), (busy, bytes));
			}
		}
		sample
	}

	/// Per thread (role: name) since `earlier`: CPU time and wake-ups, busiest first.
	pub fn threads_since(&self, earlier: &Sample) -> Vec<(String, Duration, u64)> {
		let mut out: Vec<(String, Duration, u64)> = self
			.threads
			.iter()
			.map(|(k, (cpu, sw))| {
				let (cpu0, sw0) = earlier.threads.get(k).copied().unwrap_or_default();
				(k.clone(), cpu.saturating_sub(cpu0), sw.saturating_sub(sw0))
			})
			.filter(|(_, cpu, sw)| !cpu.is_zero() || *sw > 0)
			.collect();
		out.sort_by(|a, b| b.1.cmp(&a.1));
		out
	}

	/// By role since `earlier`: GPU busy time, and the GPU memory now (bytes).
	pub fn gpu_since(&self, earlier: &Sample) -> Vec<(String, Duration, u64)> {
		let mut out: Vec<(String, Duration, u64)> = self
			.gpu
			.iter()
			.map(|(role, (busy, bytes))| {
				let busy0 = earlier.gpu.get(role).map_or(Duration::ZERO, |g| g.0);
				(role.clone(), busy.saturating_sub(busy0), *bytes)
			})
			// Not the driver's helper clients (nothing allocated, nothing run).
			.filter(|(_, busy, bytes)| *busy >= Duration::from_millis(1) || *bytes >= 1 << 20)
			.collect();
		out.sort();
		out
	}
}
