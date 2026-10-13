//! `coui://` resources: the files the UI loads, served from the game's archives.
//!
//! The UI addresses everything as `coui://<path>`, where `<path>` is an archive path:
//! `coui://ui/main.html`, `coui://gfx/ui/...png`, `coui://gfx/fonts/...ttf`. A few
//! vanilla references point at loose files beside the archives instead
//! (`coui://data/gfx/fonts/cinzel/...`, relative to the game directory).
//!
//! Lookup order (probed 2026-10-13): the archives, last-loaded first (so a mod archive
//! shadows vanilla), with exact casing (`coui://ui/MAIN.html` fails in the original);
//! then the game directory on disk, case-insensitively as on Windows. A query
//! (`?reload=…`) is ignored. (Chromium lowercases the first segment, the URL host; no
//! UI file uses an uppercase one.)
//!
//! [`UiFiles`] is `Send + Sync`: the browser serves `coui://` requests on its own
//! threads, never waiting on the game thread.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use bb_engine::DatArchive;

/// The URL scheme the UI uses.
pub const SCHEME: &str = "coui";

/// The UI's entry page.
pub const MAIN_PAGE: &str = "coui://ui/main.html";

/// The UI's file system: archives in load order, plus the game directory.
pub struct UiFiles {
	archives: Mutex<Vec<DatArchive>>,
	/// The game directory (`coui://data/...` resolves below it), if known.
	game_dir: Option<PathBuf>,
}

impl UiFiles {
	/// Open `archives` (in load order: a later one overrides an earlier one). Loose
	/// files are looked up below `game_dir`.
	pub fn open(archives: &[PathBuf], game_dir: Option<PathBuf>) -> io::Result<Self> {
		let archives = archives
			.iter()
			.map(DatArchive::open)
			.collect::<io::Result<Vec<_>>>()?;
		Ok(UiFiles {
			archives: Mutex::new(archives),
			game_dir,
		})
	}

	/// Open an install's `data_*.dat` archives (`data_dir` is the game's `data/`).
	/// With `mods`, its `mod_*.zip` archives load after them, as the original orders
	/// them.
	pub fn open_install(data_dir: &Path, mods: bool) -> io::Result<Self> {
		let mut archives = bb_engine::harness::data_archives(data_dir)?;
		if mods {
			let mut zips: Vec<PathBuf> = std::fs::read_dir(data_dir)?
				.flatten()
				.map(|e| e.path())
				.filter(|p| {
					p.file_name().is_some_and(|n| {
						let n = n.to_string_lossy();
						n.starts_with("mod_") && n.ends_with(".zip")
					})
				})
				.collect();
			zips.sort();
			archives.extend(zips);
		}
		Self::open(&archives, data_dir.parent().map(Path::to_path_buf))
	}

	/// The bytes of `path` (an archive path such as `ui/main.html`), or `None`.
	pub fn read(&self, path: &str) -> Option<Vec<u8>> {
		let path = path.trim_start_matches('/');
		if path.is_empty() {
			return None;
		}
		{
			let mut archives = self.archives.lock().unwrap_or_else(|e| e.into_inner());
			for archive in archives.iter_mut().rev() {
				if let Some(bytes) = archive.read(path) {
					return Some(bytes);
				}
			}
		}
		self.read_loose(path)
	}

	/// A file below the game directory, matching each segment case-insensitively if
	/// the exact name is missing. Never leaves the game directory.
	fn read_loose(&self, path: &str) -> Option<Vec<u8>> {
		let mut current = self.game_dir.clone()?;
		for segment in path.split('/') {
			if segment.is_empty() || segment == "." || segment == ".." {
				return None;
			}
			let exact = current.join(segment);
			current = if exact.exists() {
				exact
			} else {
				std::fs::read_dir(&current)
					.ok()?
					.flatten()
					.find(|e| {
						e.file_name()
							.to_string_lossy()
							.eq_ignore_ascii_case(segment)
					})?
					.path()
			};
		}
		std::fs::read(current).ok()
	}
}

/// The archive path a `coui://` URL names: scheme, query and fragment stripped,
/// percent-escapes decoded and `.`/`..` segments resolved. `None` for another scheme.
pub fn url_to_path(url: &str) -> Option<String> {
	let rest = url
		.strip_prefix("coui://")
		.or_else(|| url.strip_prefix("coui:"))?;
	let rest = rest.split(['?', '#']).next().unwrap_or("");
	let decoded = percent_decode(rest);
	let mut segments: Vec<&str> = Vec::new();
	for segment in decoded.split('/') {
		match segment {
			"" | "." => {}
			".." => {
				segments.pop();
			}
			s => segments.push(s),
		}
	}
	Some(segments.join("/"))
}

fn percent_decode(text: &str) -> String {
	let bytes = text.as_bytes();
	let mut out = Vec::with_capacity(bytes.len());
	let mut i = 0;
	let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
	while i < bytes.len() {
		if bytes[i] == b'%'
			&& i + 2 < bytes.len()
			&& let (Some(high), Some(low)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
		{
			out.push(high << 4 | low);
			i += 3;
			continue;
		}
		out.push(bytes[i]);
		i += 1;
	}
	String::from_utf8_lossy(&out).into_owned()
}

/// The MIME type for a path, by extension.
pub fn mime_type(path: &str) -> &'static str {
	let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
	match ext.as_str() {
		"html" | "htm" => "text/html",
		"js" => "text/javascript",
		"css" => "text/css",
		"json" => "application/json",
		"png" => "image/png",
		"jpg" | "jpeg" => "image/jpeg",
		"gif" => "image/gif",
		"svg" => "image/svg+xml",
		"webp" => "image/webp",
		"ttf" => "font/ttf",
		"otf" => "font/otf",
		"woff" => "font/woff",
		"woff2" => "font/woff2",
		"ogg" => "audio/ogg",
		"wav" => "audio/wav",
		"mp3" => "audio/mpeg",
		"txt" => "text/plain",
		_ => "application/octet-stream",
	}
}

#[cfg(test)]
mod tests {
	use super::{mime_type, url_to_path};

	#[test]
	fn urls_map_to_archive_paths() {
		assert_eq!(
			url_to_path("coui://ui/main.html").as_deref(),
			Some("ui/main.html")
		);
		assert_eq!(
			url_to_path("coui://ui/screens/../globals.css?v=2#x").as_deref(),
			Some("ui/globals.css")
		);
		assert_eq!(
			url_to_path("coui://gfx/ui/a%20b.png").as_deref(),
			Some("gfx/ui/a b.png")
		);
		assert_eq!(url_to_path("https://example.com/"), None);
	}

	#[test]
	fn mime_types_by_extension() {
		assert_eq!(mime_type("ui/main.html"), "text/html");
		assert_eq!(mime_type("gfx/fonts/x.TTF"), "font/ttf");
	}
}
