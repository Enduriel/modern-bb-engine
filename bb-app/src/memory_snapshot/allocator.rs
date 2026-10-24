//! The C allocator's own view of the game's process: what it hands out, and what it
//! holds free without giving it back to the system. glibc's `mallinfo2` and, per arena,
//! `malloc_info` (its XML, kept whole as `malloc-info.xml`); macOS's malloc zones'
//! statistics; nothing on Windows yet.

use std::path::Path;

use super::json::obj;
use super::{Part, mib};

/// One glibc arena from `malloc_info`.
#[cfg_attr(not(all(target_os = "linux", target_env = "gnu")), allow(dead_code))]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Arena {
	pub nr: u64,
	/// The memory it has from the system now.
	pub system: u64,
	/// Free chunks it holds: fast bins and the rest.
	pub free_fast: u64,
	pub free_rest: u64,
}

pub fn collect(dir: &Path) -> Part {
	let mut part = Part::new("allocator");
	imp::collect(dir, &mut part);
	part
}

/// The arenas in `malloc_info`'s XML (`<heap nr="0">…<system type="current" size=…/>`).
#[cfg_attr(not(all(target_os = "linux", target_env = "gnu")), allow(dead_code))]
pub fn arenas(xml: &str) -> Vec<Arena> {
	let attr = |tag: &str, name: &str| -> Option<u64> {
		let start = tag.find(&format!("{name}=\""))? + name.len() + 2;
		tag[start..].split('"').next()?.parse().ok()
	};
	let mut arenas = Vec::new();
	for heap in xml.split("<heap nr=").skip(1) {
		let heap = heap.split("</heap>").next().unwrap_or(heap);
		let nr = heap.trim_start_matches('"').split('"').next().and_then(|n| n.parse().ok()).unwrap_or(0);
		let mut arena = Arena { nr, ..Default::default() };
		for tag in heap.split('<') {
			if tag.starts_with("system type=\"current\"") {
				arena.system = attr(tag, "size").unwrap_or(0);
			} else if tag.starts_with("total type=\"fast\"") {
				arena.free_fast = attr(tag, "size").unwrap_or(0);
			} else if tag.starts_with("total type=\"rest\"") {
				arena.free_rest = attr(tag, "size").unwrap_or(0);
			}
		}
		arenas.push(arena);
	}
	arenas
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
mod imp {
	use std::path::Path;

	use super::super::json::Json;
	use super::*;

	#[repr(C)]
	struct Mallinfo2 {
		arena: usize,
		ordblks: usize,
		smblks: usize,
		hblks: usize,
		hblkhd: usize,
		usmblks: usize,
		fsmblks: usize,
		uordblks: usize,
		fordblks: usize,
		keepcost: usize,
	}

	unsafe extern "C" {
		fn mallinfo2() -> Mallinfo2;
		fn malloc_info(options: libc::c_int, stream: *mut libc::FILE) -> libc::c_int;
	}

	pub(super) fn collect(dir: &Path, part: &mut Part) {
		// SAFETY: `mallinfo2` only reads the allocator's counters.
		let m = unsafe { mallinfo2() };
		let in_use = m.uordblks + m.hblkhd;
		let mut json = obj()
			.with("allocator", "glibc")
			.with("in_use", in_use)
			.with("free_held", m.fordblks)
			.with("main_arena_from_system", m.arena)
			.with("mmapped_chunks", m.hblks)
			.with("mmapped_bytes", m.hblkhd)
			.with("free_chunks", m.ordblks)
			.with("fastbin_chunks", m.smblks)
			.with("fastbin_free_bytes", m.fsmblks)
			.with("releasable_top", m.keepcost);
		part.line(format!(
			"glibc malloc (the game's process): {} in use ({} of it mmapped on its own), {} free but held",
			mib(in_use as u64),
			mib(m.hblkhd as u64),
			mib(m.fordblks as u64)
		));
		match write_malloc_info(&dir.join("malloc-info.xml")) {
			Ok(xml) => {
				part.file("malloc-info.xml");
				let arenas = arenas(&xml);
				let system: u64 = arenas.iter().map(|a| a.system).sum();
				let free: u64 = arenas.iter().map(|a| a.free_fast + a.free_rest).sum();
				part.line(format!("  {} arenas: {} from the system, {} of it free", arenas.len(), mib(system), mib(free)));
				let mut by_size = arenas.clone();
				by_size.sort_by_key(|a| std::cmp::Reverse(a.system));
				for a in by_size.iter().take(8) {
					part.line(format!("    arena {}: {} from the system, {} free", a.nr, mib(a.system), mib(a.free_fast + a.free_rest)));
				}
				let rows = arenas
					.iter()
					.map(|a| obj().with("nr", a.nr).with("system", a.system).with("free_fast", a.free_fast).with("free_rest", a.free_rest))
					.collect();
				json = json.with("arenas", Json::Array(rows));
			}
			Err(e) => part.error(format!("malloc_info: {e}")),
		}
		part.json = json;
	}

	/// Write glibc's `malloc_info` to `path`, and return it.
	fn write_malloc_info(path: &Path) -> Result<String, String> {
		let name = std::ffi::CString::new(path.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;
		// SAFETY: a stream of our own, closed after the call writes to it.
		unsafe {
			let file = libc::fopen(name.as_ptr(), c"w".as_ptr());
			if file.is_null() {
				return Err(std::io::Error::last_os_error().to_string());
			}
			let written = malloc_info(0, file);
			libc::fclose(file);
			if written != 0 {
				return Err("it failed".into());
			}
		}
		std::fs::read_to_string(path).map_err(|e| e.to_string())
	}
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
mod imp {
	use std::path::Path;

	use super::*;

	pub(super) fn collect(_dir: &Path, part: &mut Part) {
		match bb_engine::heap::usage() {
			Some(u) => {
				part.line(format!("malloc (the game's process): {} in use, {} free but held", mib(u.in_use as u64), mib(u.free as u64)));
				part.json = obj().with("allocator", "system").with("in_use", u.in_use).with("free_held", u.free);
			}
			None => part.error("not available on this system yet".to_string()),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn reads_malloc_infos_arenas() {
		let xml = r#"<malloc version="1">
<heap nr="0">
<sizes>
<size from="17" to="32" total="64" count="2"/>
</sizes>
<total type="fast" count="2" size="64"/>
<total type="rest" count="5" size="1000"/>
<system type="current" size="135168"/>
<system type="max" size="135168"/>
<aspace type="total" size="135168"/>
</heap>
<heap nr="1">
<total type="fast" count="0" size="0"/>
<total type="rest" count="1" size="20"/>
<system type="current" size="4096"/>
</heap>
<total type="fast" count="2" size="64"/>
<system type="current" size="139264"/>
</malloc>"#;
		let a = arenas(xml);
		assert_eq!(a, [
			Arena { nr: 0, system: 135168, free_fast: 64, free_rest: 1000 },
			Arena { nr: 1, system: 4096, free_fast: 0, free_rest: 20 },
		]);
	}

	#[cfg(all(target_os = "linux", target_env = "gnu"))]
	#[test]
	fn writes_this_process_malloc_info() {
		let dir = std::env::temp_dir().join(format!("bb-malloc-info-{}", std::process::id()));
		std::fs::create_dir_all(&dir).unwrap();
		let part = collect(&dir);
		assert!(part.errors.is_empty(), "{:?}", part.errors);
		let xml = std::fs::read_to_string(dir.join("malloc-info.xml")).unwrap();
		assert!(!arenas(&xml).is_empty(), "{xml}");
		let _ = std::fs::remove_dir_all(&dir);
	}
}
