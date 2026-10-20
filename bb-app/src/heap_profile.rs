//! Heap profiles of the game's process under jemalloc with profiling on
//! (`LD_PRELOAD=libjemalloc.so.2 MALLOC_CONF=prof:true`), for `parity/perf/heap_owners.py`
//! (who holds the engine's heap). CEF doesn't run under a preloaded jemalloc, so this is
//! for builds without the `ui` feature. Without jemalloc loaded, nothing is dumped.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

unsafe extern "C" {
	fn dlsym(handle: *mut std::ffi::c_void, symbol: *const std::ffi::c_char) -> *mut std::ffi::c_void;
}

type Mallctl = unsafe extern "C" fn(
	name: *const std::ffi::c_char,
	old: *mut std::ffi::c_void,
	old_len: *mut usize,
	new: *mut std::ffi::c_void,
	new_len: usize,
) -> std::ffi::c_int;

/// Dump a heap profile of this moment to `path`; an error says why it couldn't.
pub fn dump(path: &Path) -> Result<(), String> {
	// `RTLD_DEFAULT`: the symbol wherever it is loaded (the preloaded jemalloc).
	let mallctl = unsafe { dlsym(std::ptr::null_mut(), c"mallctl".as_ptr()) };
	if mallctl.is_null() {
		return Err("jemalloc isn't loaded".to_string());
	}
	let mallctl: Mallctl = unsafe { std::mem::transmute(mallctl) };
	let name = std::ffi::CString::new(path.to_string_lossy().into_owned()).map_err(|e| e.to_string())?;
	let mut file = name.as_ptr();
	let done = unsafe {
		mallctl(
			c"prof.dump".as_ptr(),
			std::ptr::null_mut(),
			std::ptr::null_mut(),
			(&raw mut file).cast(),
			std::mem::size_of::<*const std::ffi::c_char>(),
		)
	};
	if done != 0 {
		return Err(format!("mallctl error {done}"));
	}
	Ok(())
}

/// With `BB_HEAP_PROFILE_DIR` set: a heap profile every `BB_HEAP_PROFILE_SECS` seconds
/// (default 10) into that folder, named by its number and the seconds since the start,
/// from a thread of its own.
pub fn spawn_periodic_from_env() {
	let Some(dir) = std::env::var_os("BB_HEAP_PROFILE_DIR").map(PathBuf::from) else { return };
	let every = std::env::var("BB_HEAP_PROFILE_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(10.0);
	let start = Instant::now();
	let spawned = std::thread::Builder::new().name("bb-heap-profile".to_string()).spawn(move || {
		for index in 0.. {
			std::thread::sleep(Duration::from_secs_f64(every));
			let path = dir.join(format!("{index:03}-t{:.0}.heap", start.elapsed().as_secs_f64()));
			if let Err(e) = dump(&path) {
				eprintln!("BB_HEAP_PROFILE_DIR: {e}");
				return;
			}
		}
	});
	if let Err(e) = spawned {
		eprintln!("BB_HEAP_PROFILE_DIR: {e}");
	}
}
