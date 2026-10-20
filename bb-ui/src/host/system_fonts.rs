//! The system's Windows fonts with whole-pixel advances, on macOS.
//!
//! The page's generic families are Windows' defaults (Times New Roman, Arial, Courier
//! New: `probes/font_families.js`), and macOS has those fonts, but there their glyphs
//! advance by fractions of a pixel (Courier New's sample 288px where the original's is
//! 300). Fonts the page loads get whole-pixel advances from the `coui://` handler
//! ([`whole_pixel`]); these come from the system, so the renderer process registers
//! its own [`whole_pixel`] variants of them before Chromium starts (CoreText, for
//! this process only), under names of their own ("BB Arial"): two fonts with one
//! PostScript name can't both be registered. Then:
//!
//! - the generic families name the variants ([`generic`]). The standard one is also
//!   the initial `font-family`, which scripts can read, so the root keeps the font's
//!   own name with a zero-specificity rule, as the initial value had it;
//! - the page's font faces ([`page_script`], added before any page script runs) give
//!   the variants the fonts' own names, so `font-family: Arial` gets them too.
//!
//! A font the system lacks is left alone: its family keeps the system's lookup.

use std::ffi::c_void;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use crate::whole_pixel;

/// The families.
const FAMILIES: [&str; 3] = [STANDARD, "Arial", "Courier New"];

/// The standard family, which text that names no font it finds gets.
pub(super) const STANDARD: &str = "Times New Roman";

/// Each family's faces: the file name's suffix, `font-weight` and `font-style`.
const FACES: [(&str, &str, &str); 4] = [
	("", "normal", "normal"),
	(" Bold", "bold", "normal"),
	(" Italic", "normal", "italic"),
	(" Bold Italic", "bold", "italic"),
];

/// Where macOS keeps the fonts.
const DIRS: [&str; 2] = ["/System/Library/Fonts/Supplemental", "/Library/Fonts"];

/// The variants' family names start so.
const PREFIX: &str = "BB ";

/// The file of `family`'s face with `suffix`, if the system has it.
fn file(family: &str, suffix: &str) -> Option<PathBuf> {
	DIRS.iter()
		.map(|dir| PathBuf::from(dir).join(format!("{family}{suffix}.ttf")))
		.find(|path| path.is_file())
}

/// The PostScript name of the variant of `family`'s face with `suffix`.
fn postscript_name(family: &str, suffix: &str) -> String {
	format!("{PREFIX}{family}{suffix}").replace(' ', "")
}

/// The family a generic family (`family`, one of Windows' defaults) should name: the
/// variant, if the system has the font.
pub(super) fn generic(family: &str) -> String {
	if file(family, "").is_some() {
		format!("{PREFIX}{family}")
	} else {
		family.to_string()
	}
}

/// The page script that adds the font faces giving the variants the fonts' own names.
pub(super) fn page_script() -> String {
	let mut faces = Vec::new();
	for family in FAMILIES {
		for (suffix, weight, style) in FACES {
			if file(family, suffix).is_some() {
				// By PostScript name: Blink on macOS doesn't find these by full name.
				let postscript = postscript_name(family, suffix);
				faces.push(format!("['{family}', \"local('{postscript}')\", '{weight}', '{style}']"));
			}
		}
	}
	format!(
		"[{}].forEach(function (f) {{\n\
		 \tvar face = new FontFace(f[0], f[1], {{ weight: f[2], style: f[3] }});\n\
		 \tdocument.fonts.add(face);\n\
		 \tface.load()['catch'](function () {{}});\n\
		 }});\n\
		 document.addEventListener('DOMContentLoaded', function () {{\n\
		 \tvar style = document.createElement('style');\n\
		 \tstyle.setAttribute('data-bb-compat', 'system-fonts');\n\
		 \tstyle.textContent = ':where(:root) {{ font-family: \"{STANDARD}\"; }}';\n\
		 \tdocument.head.insertBefore(style, document.head.firstChild);\n\
		 }});\n",
		faces.join(", ")
	)
}

/// Register the variants for this process (the renderer, before Chromium starts).
///
/// Each is made once (about 5ms and 1.3MB a face) and kept in the engine's cache
/// folder, under a name that changes with the system's file and with [`VERSION`];
/// CoreText maps registered files rather than holding them in memory.
pub(super) fn register() {
	let cache = bb_assets::cache_dir::engine_cache_dir().map(|dir| dir.join("fonts"));
	for family in FAMILIES {
		for (suffix, _, _) in FACES {
			let Some(path) = file(family, suffix) else { continue };
			let postscript = postscript_name(family, suffix);
			let registered = match &cache {
				Some(dir) => cached(dir, &path, family, suffix).is_some_and(|file| register_file(&file)),
				None => variant(&path, family, suffix).is_some_and(|font| register_data(&font)),
			};
			if !registered {
				eprintln!("bb-ui: {postscript} isn't registered; {} keeps its fractional advances", path.display());
			}
		}
	}
}

/// What the variants are made by: changes when [`whole_pixel`] or the names change.
const VERSION: u32 = 1;

/// The variant of `family`'s face with `suffix` (the system's `path`).
fn variant(path: &Path, family: &str, suffix: &str) -> Option<Vec<u8>> {
	let data = std::fs::read(path).ok()?;
	let full = format!("{PREFIX}{family}{suffix}");
	let font = whole_pixel::variant(&data)?;
	whole_pixel::renamed(&font, &format!("{PREFIX}{family}"), &full, &postscript_name(family, suffix))
}

/// The variant's file in `dir`, made if missing (replacing older ones).
fn cached(dir: &Path, path: &Path, family: &str, suffix: &str) -> Option<PathBuf> {
	let source = std::fs::metadata(path).ok()?;
	let mut hash = DefaultHasher::new();
	(VERSION, path, source.len(), source.modified().ok()).hash(&mut hash);
	let postscript = postscript_name(family, suffix);
	let name = format!("{postscript}-{:016x}.ttf", hash.finish());
	let file = dir.join(&name);
	if file.is_file() {
		return Some(file);
	}
	let font = variant(path, family, suffix)?;
	std::fs::create_dir_all(dir).ok()?;
	if let Ok(entries) = std::fs::read_dir(dir) {
		for entry in entries.flatten() {
			let old = entry.file_name().to_string_lossy().into_owned();
			if old != name && old.strip_prefix(&postscript).is_some_and(|rest| rest.starts_with('-')) {
				let _ = std::fs::remove_file(entry.path());
			}
		}
	}
	// Written whole, then moved in place: another renderer may be reading it.
	let partial = dir.join(format!("{name}.{}", std::process::id()));
	std::fs::write(&partial, font).ok()?;
	std::fs::rename(&partial, &file).ok()?;
	Some(file)
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
	fn CFDataCreate(allocator: *const c_void, bytes: *const u8, length: isize) -> *const c_void;
	fn CFURLCreateFromFileSystemRepresentation(
		allocator: *const c_void,
		path: *const u8,
		length: isize,
		directory: bool,
	) -> *const c_void;
	fn CFRelease(object: *const c_void);
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
	fn CGDataProviderCreateWithCFData(data: *const c_void) -> *const c_void;
	fn CGFontCreateWithDataProvider(provider: *const c_void) -> *const c_void;
}

#[link(name = "CoreText", kind = "framework")]
unsafe extern "C" {
	fn CTFontManagerRegisterFontsForURL(url: *const c_void, scope: u32, error: *mut *const c_void) -> bool;
	fn CTFontManagerRegisterGraphicsFont(font: *const c_void, error: *mut *const c_void) -> bool;
}

/// `kCTFontManagerScopeProcess`.
const SCOPE_PROCESS: u32 = 1;

/// Register the font file at `path` with CoreText for this process.
fn register_file(path: &Path) -> bool {
	let bytes = path.as_os_str().as_bytes();
	// SAFETY: plain CoreFoundation calls; every object created is released once.
	unsafe {
		let url = CFURLCreateFromFileSystemRepresentation(std::ptr::null(), bytes.as_ptr(), bytes.len() as isize, false);
		if url.is_null() {
			return false;
		}
		let mut error = std::ptr::null();
		let registered = CTFontManagerRegisterFontsForURL(url, SCOPE_PROCESS, &mut error);
		if !error.is_null() {
			CFRelease(error);
		}
		CFRelease(url);
		registered
	}
}

/// Register the font file `data` with CoreText for this process.
fn register_data(data: &[u8]) -> bool {
	// SAFETY: plain CoreFoundation calls; every object created is released once, and
	// CoreText retains what it registers.
	unsafe {
		let bytes = CFDataCreate(std::ptr::null(), data.as_ptr(), data.len() as isize);
		if bytes.is_null() {
			return false;
		}
		let provider = CGDataProviderCreateWithCFData(bytes);
		CFRelease(bytes);
		if provider.is_null() {
			return false;
		}
		let font = CGFontCreateWithDataProvider(provider);
		CFRelease(provider);
		if font.is_null() {
			return false;
		}
		let mut error = std::ptr::null();
		let registered = CTFontManagerRegisterGraphicsFont(font, &mut error);
		if !error.is_null() {
			CFRelease(error);
		}
		CFRelease(font);
		registered
	}
}
