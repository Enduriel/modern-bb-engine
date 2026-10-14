//! The game's processes, from `/proc`.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use super::{MappingKind, ProcessMemory, Usage};

/// `/proc`'s times are in clock ticks of `USER_HZ`, 100 on every Linux.
const TICKS_PER_SECOND: u64 = 100;

/// This process and every process it started, it first.
fn tree() -> Vec<u32> {
	let mut todo = vec![std::process::id()];
	let mut pids = Vec::new();
	while let Some(pid) = todo.pop() {
		pids.push(pid);
		todo.extend(children(pid));
	}
	pids
}

pub(super) fn tree_usage() -> Option<Usage> {
	let pids = tree();
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

pub(super) fn tree_memory() -> Vec<ProcessMemory> {
	tree().into_iter().filter_map(process_memory).collect()
}

fn process_memory(pid: u32) -> Option<ProcessMemory> {
	let smaps = std::fs::read_to_string(format!("/proc/{pid}/smaps")).ok()?;
	let mut kinds: HashMap<MappingKind, u64> = HashMap::new();
	let mut kind = MappingKind::Anonymous;
	for line in smaps.lines() {
		if let Some(kb) = line.strip_prefix("Pss:") {
			let kb: u64 = kb.split_whitespace().next().and_then(|n| n.parse().ok()).unwrap_or(0);
			*kinds.entry(kind).or_default() += kb * 1024;
		} else if is_mapping_header(line) {
			// `start-end perms offset dev inode [name]`
			kind = mapping_kind(line.split_whitespace().nth(5).unwrap_or(""));
		}
	}
	let mut kinds: Vec<(MappingKind, u64)> = kinds.into_iter().filter(|&(_, b)| b > 0).collect();
	kinds.sort_by(|a, b| b.1.cmp(&a.1));
	let (vram, gtt) = gpu_memory(pid);
	Some(ProcessMemory {
		pid,
		role: role(pid),
		kinds,
		vram,
		gtt,
	})
}

/// Whether an `smaps` line starts a mapping (`7f12…-7f13… r-xp …`) rather than giving
/// one of its fields (`Pss:   12 kB`).
fn is_mapping_header(line: &str) -> bool {
	line.split_once(' ')
		.is_some_and(|(range, _)| range.contains('-') && range.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-'))
}

/// What a mapping of this name holds (`smaps`' last field; empty for anonymous memory).
fn mapping_kind(name: &str) -> MappingKind {
	const DRIVERS: [&str; 9] = ["/dev/dri", "libvulkan_", "libgallium", "libLLVM", "libdrm", "libnvidia", "libGLX_", "libEGL_", "amdgpu"];
	match name {
		"[heap]" => MappingKind::Heap,
		"" => MappingKind::Anonymous,
		n if n.starts_with("[stack") => MappingKind::Stacks,
		n if n.starts_with("[anon:v8") => MappingKind::V8,
		n if n.starts_with("[anon:partition_alloc") => MappingKind::PartitionAlloc,
		n if n.starts_with('[') => MappingKind::Anonymous,
		n if n.contains("libcef.so") => MappingKind::Libcef,
		n if DRIVERS.iter().any(|d| n.contains(d)) => MappingKind::GpuDriver,
		n if n.starts_with("/memfd:") || n.starts_with("/dev/shm") || n.starts_with("/SYSV") => MappingKind::Shared,
		_ => MappingKind::Files,
	}
}

/// The GPU memory the process allocated, in video memory and in GTT, from its DRM file
/// descriptors' `fdinfo` (each client once: one can be open on several descriptors).
/// amdgpu and most drivers name the regions `drm-memory-vram`/`-gtt`; Intel's
/// `drm-total-local0`/`-system0`.
fn gpu_memory(pid: u32) -> (u64, u64) {
	let Ok(fds) = std::fs::read_dir(format!("/proc/{pid}/fdinfo")) else { return (0, 0) };
	let mut clients = HashSet::new();
	let (mut vram, mut gtt) = (0, 0);
	for fd in fds.flatten() {
		let Ok(info) = std::fs::read_to_string(fd.path()) else { continue };
		let field = |key: &str| info.lines().find_map(|l| l.strip_prefix(key)?.trim().strip_prefix(':').map(str::trim));
		let Some(client) = field("drm-client-id") else { continue };
		if !clients.insert(client.to_string()) {
			continue;
		}
		let bytes = |key: &str| field(key).map_or(0, parse_size);
		vram += bytes("drm-memory-vram").max(bytes("drm-total-local0"));
		gtt += bytes("drm-memory-gtt").max(bytes("drm-total-system0"));
	}
	(vram, gtt)
}

/// `123 KiB`, `4 MiB` or a bare number of bytes.
fn parse_size(text: &str) -> u64 {
	let mut parts = text.split_whitespace();
	let n: u64 = parts.next().and_then(|n| n.parse().ok()).unwrap_or(0);
	match parts.next() {
		Some("KiB") => n << 10,
		Some("MiB") => n << 20,
		Some("GiB") => n << 30,
		_ => n,
	}
}

/// `game` for the game's own process; for CEF's, its `--type` (and a utility's service).
fn role(pid: u32) -> String {
	let Ok(cmdline) = std::fs::read(format!("/proc/{pid}/cmdline")) else { return "?".to_string() };
	let args: Vec<&[u8]> = cmdline.split(|&b| b == 0).collect();
	let value = |key: &str| {
		args.iter()
			.find_map(|a| a.strip_prefix(key.as_bytes()))
			.map(|v| String::from_utf8_lossy(v).into_owned())
	};
	match value("--type=") {
		None => "game".to_string(),
		Some(kind) => match value("--utility-sub-type=") {
			// `storage.mojom.StorageService` → `utility:StorageService`
			Some(service) => format!("{kind}:{}", service.rsplit('.').next().unwrap_or(&service)),
			None => kind,
		},
	}
}

#[cfg(test)]
mod tests {
	#[test]
	fn reads_this_process() {
		let usage = super::tree_usage().expect("usage");
		assert!(usage.processes >= 1);
		assert!(usage.pss_bytes > 0, "a process has memory");
	}

	#[test]
	fn breaks_this_process_down() {
		let all = super::tree_memory();
		let me = all.iter().find(|p| p.pid == std::process::id()).expect("this process");
		assert_eq!(me.role, "game");
		assert!(me.pss() > 0);
		let kinds: Vec<_> = me.kinds.iter().map(|(k, _)| *k).collect();
		assert!(kinds.contains(&super::MappingKind::Files), "its own binary: {kinds:?}");
	}

	#[test]
	fn names_mappings() {
		use super::{MappingKind, is_mapping_header, mapping_kind, parse_size};
		assert!(is_mapping_header("7f12a000-7f13b000 r-xp 00000000 00:1d 41600   /usr/lib/libcef.so"));
		assert!(!is_mapping_header("Pss:                  12 kB"));
		assert_eq!(mapping_kind("/opt/cef/libcef.so"), MappingKind::Libcef);
		assert_eq!(mapping_kind("[anon:v8-sandbox]"), MappingKind::V8);
		assert_eq!(mapping_kind("[anon:partition_alloc]"), MappingKind::PartitionAlloc);
		assert_eq!(mapping_kind("/usr/lib/x86_64-linux-gnu/libvulkan_radeon.so"), MappingKind::GpuDriver);
		assert_eq!(mapping_kind("/memfd:shared-memory-region (deleted)"), MappingKind::Shared);
		assert_eq!(parse_size("860196 KiB"), 860_196 << 10);
		assert_eq!(parse_size("2 MiB"), 2 << 20);
	}
}
