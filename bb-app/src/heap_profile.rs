//! Heap profiles of the game's process under jemalloc with profiling on
//! (`LD_PRELOAD=libjemalloc.so.2 MALLOC_CONF=prof:true`), for `parity/perf/heap_owners.py`
//! (who holds the engine's heap). CEF doesn't run under a preloaded jemalloc, so this is
//! for builds without the `ui` feature. Without jemalloc loaded, nothing is dumped. On
//! Windows (no `dlsym`, no preloading) there are none.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[cfg(unix)]
unsafe extern "C" {
	fn dlsym(handle: *mut std::ffi::c_void, symbol: *const std::ffi::c_char) -> *mut std::ffi::c_void;
}

/// `RTLD_DEFAULT`: a symbol wherever it is loaded (the preloaded jemalloc).
#[cfg(target_os = "linux")]
const RTLD_DEFAULT: *mut std::ffi::c_void = std::ptr::null_mut();
#[cfg(all(unix, not(target_os = "linux")))]
const RTLD_DEFAULT: *mut std::ffi::c_void = -2isize as *mut std::ffi::c_void;

type Mallctl = unsafe extern "C" fn(
	name: *const std::ffi::c_char,
	old: *mut std::ffi::c_void,
	old_len: *mut usize,
	new: *mut std::ffi::c_void,
	new_len: usize,
) -> std::ffi::c_int;

/// Dump a heap profile of this moment to `path`; an error says why it couldn't.
pub fn dump(path: &Path) -> Result<(), String> {
	let mallctl = mallctl().ok_or("jemalloc isn't loaded")?;
	let name = std::ffi::CString::new(path.to_string_lossy().into_owned()).map_err(|e| e.to_string())?;
	let mut file = name.as_ptr();
	// SAFETY: `prof.dump` takes a `const char *` path as its new value, which `file`
	// points to (a NUL-terminated string alive for the call); it returns nothing.
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

/// jemalloc's `mallctl`, where it is loaded.
#[cfg(unix)]
fn mallctl() -> Option<Mallctl> {
	// SAFETY: `dlsym` with `RTLD_DEFAULT` and a NUL-terminated name only looks the symbol up.
	let found = unsafe { dlsym(RTLD_DEFAULT, c"mallctl".as_ptr()) };
	// SAFETY: jemalloc's `mallctl` has `Mallctl`'s signature.
	(!found.is_null()).then(|| unsafe { std::mem::transmute::<*mut std::ffi::c_void, Mallctl>(found) })
}

#[cfg(not(unix))]
fn mallctl() -> Option<Mallctl> {
	None
}

/// The time between heap profiles, from `BB_HEAP_PROFILE_SECS`: 10 s unless it is a
/// positive, finite number of seconds.
fn interval(secs: Option<&str>) -> Duration {
	secs.and_then(|s| s.trim().parse::<f64>().ok())
		.and_then(|s| Duration::try_from_secs_f64(s).ok())
		.filter(|d| !d.is_zero())
		.unwrap_or(Duration::from_secs(10))
}

/// With `BB_HEAP_PROFILE_DIR` set: a heap profile every `BB_HEAP_PROFILE_SECS` seconds
/// (default 10) into that folder, named by its number and the seconds since the start,
/// from a thread of its own.
pub fn spawn_periodic_from_env() {
	let Some(dir) = std::env::var_os("BB_HEAP_PROFILE_DIR").map(PathBuf::from) else { return };
	let every = interval(std::env::var("BB_HEAP_PROFILE_SECS").ok().as_deref());
	let start = Instant::now();
	let spawned = std::thread::Builder::new().name("bb-heap-profile".to_string()).spawn(move || {
		for index in 0.. {
			std::thread::sleep(every);
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

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_interval_is_ten_seconds_unless_a_positive_finite_one_is_set() {
		assert_eq!(interval(Some("2.5")), Duration::from_millis(2500));
		for bad in [None, Some(""), Some("soon"), Some("-1"), Some("NaN"), Some("inf"), Some("0"), Some("1e300")] {
			assert_eq!(interval(bad), Duration::from_secs(10), "{bad:?}");
		}
	}
}
