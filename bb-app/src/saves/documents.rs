//! The player's Documents folder on Windows, from the shell's known folders
//! (`FOLDERID_Documents`), where the original's `Battle Brothers` folder is: wherever
//! Documents was moved (OneDrive's backup, another drive), not `%USERPROFILE%\Documents`.

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;

use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{FOLDERID_Documents, KF_FLAG_DEFAULT, SHGetKnownFolderPath};

/// The Documents folder, or why the shell gave none.
pub fn documents_dir() -> Result<PathBuf, String> {
	// SAFETY: the GUID is a static; on success the shell returns a NUL-terminated
	// string it allocated, read once here and freed with `CoTaskMemFree` as documented.
	unsafe {
		let path = SHGetKnownFolderPath(&FOLDERID_Documents, KF_FLAG_DEFAULT, None)
			.map_err(|e| format!("no Documents folder from the shell ({e})"))?;
		let wide = path.as_wide();
		let dir = PathBuf::from(OsString::from_wide(wide));
		CoTaskMemFree(Some(path.0 as *const _));
		Ok(dir)
	}
}
