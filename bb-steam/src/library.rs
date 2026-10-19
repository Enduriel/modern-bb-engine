//! A shared library loaded at run time, and its symbols: the platform layer under the
//! Steam binding (`dlopen`/`dlsym` on Unix, `LoadLibraryW`/`GetProcAddress` on Windows).

use std::ffi::{CString, c_void};
use std::path::Path;

/// A loaded library, unloaded when dropped. Not `Send`: Steam's API is used from the
/// thread that loaded it.
pub struct Library {
	handle: *mut c_void,
}

impl Library {
	/// Load the library at `path` (a path, never a bare name, so the system's search
	/// path plays no part).
	pub fn open(path: &Path) -> Result<Library, String> {
		let handle = sys::open(path)?;
		Ok(Library { handle })
	}

	/// The address of the exported symbol `name`, if the library has it.
	pub fn symbol(&self, name: &str) -> Option<*const c_void> {
		let name = CString::new(name).ok()?;
		let address = sys::symbol(self.handle, &name);
		(!address.is_null()).then_some(address as *const c_void)
	}
}

impl Drop for Library {
	fn drop(&mut self) {
		sys::close(self.handle);
	}
}

#[cfg(unix)]
mod sys {
	use std::ffi::{CStr, CString, c_void};
	use std::os::unix::ffi::OsStrExt;
	use std::path::Path;

	pub fn open(path: &Path) -> Result<*mut c_void, String> {
		let c_path = CString::new(path.as_os_str().as_bytes()).map_err(|_| format!("{}: a NUL in the path", path.display()))?;
		// SAFETY: a valid C string; loading runs the library's initializers, which is
		// what loading it means (the caller chose the file).
		let handle = unsafe { libc::dlopen(c_path.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
		if handle.is_null() {
			// SAFETY: dlerror returns a C string owned by libc, or null.
			let error = unsafe { libc::dlerror() };
			let message = if error.is_null() { "unknown error".into() } else { unsafe { CStr::from_ptr(error) }.to_string_lossy().into_owned() };
			return Err(format!("{}: {message}", path.display()));
		}
		Ok(handle)
	}

	pub fn symbol(handle: *mut c_void, name: &CStr) -> *mut c_void {
		// SAFETY: a handle dlopen returned, and a valid C string.
		unsafe { libc::dlsym(handle, name.as_ptr()) }
	}

	pub fn close(handle: *mut c_void) {
		// SAFETY: a handle dlopen returned, closed once.
		unsafe { libc::dlclose(handle) };
	}
}

#[cfg(windows)]
mod sys {
	use std::ffi::{CStr, c_void};
	use std::os::windows::ffi::OsStrExt;
	use std::path::Path;

	#[link(name = "kernel32")]
	unsafe extern "system" {
		fn LoadLibraryW(name: *const u16) -> *mut c_void;
		fn GetProcAddress(module: *mut c_void, name: *const std::ffi::c_char) -> *mut c_void;
		fn FreeLibrary(module: *mut c_void) -> i32;
		fn GetLastError() -> u32;
	}

	pub fn open(path: &Path) -> Result<*mut c_void, String> {
		let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
		// SAFETY: a NUL-terminated wide string.
		let handle = unsafe { LoadLibraryW(wide.as_ptr()) };
		if handle.is_null() {
			// SAFETY: no arguments; reads the thread's last error.
			return Err(format!("{}: LoadLibraryW failed (error {})", path.display(), unsafe { GetLastError() }));
		}
		Ok(handle)
	}

	pub fn symbol(handle: *mut c_void, name: &CStr) -> *mut c_void {
		// SAFETY: a module LoadLibraryW returned, and a valid C string.
		unsafe { GetProcAddress(handle, name.as_ptr()) }
	}

	pub fn close(handle: *mut c_void) {
		// SAFETY: a module LoadLibraryW returned, freed once.
		unsafe { FreeLibrary(handle) };
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_missing_library_is_an_error() {
		let err = Library::open(Path::new("/nonexistent/libsteam_api.so")).err().expect("no such file");
		assert!(err.contains("nonexistent"), "{err}");
	}
}
