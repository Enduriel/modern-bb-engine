//! macOS: `bb` as an app bundle, the layout CEF expects there.
//!
//! ```text
//! bb.app/Contents/
//!   Info.plist
//!   MacOS/bb, libsteam_api.dylib
//!   Resources/bb.icns                                   (the game's icon, [`crate::icns`])
//!   Frameworks/Chromium Embedded Framework.framework   (English locale only)
//!   Frameworks/bb Helper.app/Contents/{Info.plist, MacOS/bb Helper}
//!   Frameworks/bb Helper (Renderer).app/Contents/{Info.plist, MacOS/bb Helper (Renderer)}
//! ```
//!
//! CEF's child processes (the page's renderer and the rest) run `bb Helper`, a copy of
//! `bb` (each carries its own bundle's signature, so not a link), from a bundle of its own
//! that keeps them out of the Dock (`LSUIElement`). Chromium starts the page's renderer
//! from the `(Renderer)` one, other children from the plain one (checked: without the
//! `(Renderer)` helper the page never loads); the `(GPU)`, `(Plugin)` and `(Alerts)` ones
//! CEF's own layout has go unused, as the host keeps the GPU in its process and has
//! neither plugins nor notifications. Each finds the framework from where
//! they are (`bb-ui`'s `host/mac.rs`). The bundle is signed ad hoc afterwards, as Apple
//! Silicon needs every program signed and copying left the bundle's seal out of date.

use std::fs;
use std::path::Path;

use crate::steam_library::Steam;

/// The bundle's and the helper's names.
pub const APP: &str = "bb.app";
const HELPER: &str = "bb Helper";
/// The helpers' kinds, as Chromium names them (`""`: the plain one).
const HELPER_KINDS: [&str; 2] = ["", " (Renderer)"];
/// The framework's folder in CEF's distribution and in the bundle.
const FRAMEWORK: &str = "Chromium Embedded Framework.framework";
/// The bundle identifier (reverse DNS, no domain of anyone else's).
const IDENTIFIER: &str = "local.battle-brothers-modern.bb";

/// An `Info.plist` for `executable`; a helper's keeps out of the Dock and has no icon.
fn info_plist(executable: &str, identifier: &str, helper: bool) -> String {
	let ui_element = if helper { "\t<key>LSUIElement</key>\n\t<string>1</string>\n" } else { "" };
	let icon = if helper { String::new() } else { format!("\t<key>CFBundleIconFile</key>\n\t<string>{}</string>\n", crate::icns::FILE) };
	format!(
		r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleDevelopmentRegion</key>
	<string>en</string>
	<key>CFBundleDisplayName</key>
	<string>{executable}</string>
	<key>CFBundleExecutable</key>
	<string>{executable}</string>
	<key>CFBundleIdentifier</key>
	<string>{identifier}</string>
	<key>CFBundleInfoDictionaryVersion</key>
	<string>6.0</string>
	<key>CFBundleName</key>
	<string>{executable}</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
	<key>CFBundleShortVersionString</key>
	<string>{version}</string>
	<key>CFBundleVersion</key>
	<string>{version}</string>
	<key>LSMinimumSystemVersion</key>
	<string>12.0</string>
	<key>NSHighResolutionCapable</key>
	<true/>
	<key>NSSupportsAutomaticGraphicsSwitching</key>
	<true/>
{icon}{ui_element}</dict>
</plist>
"#,
		version = env!("CARGO_PKG_VERSION"),
	)
}

/// Copy the folder `from` to `to`, keeping symbolic links as links.
fn copy_dir(from: &Path, to: &Path) -> Result<(), String> {
	fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;
	for entry in fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))?.flatten() {
		let (source, target) = (entry.path(), to.join(entry.file_name()));
		let kind = entry.file_type().map_err(|e| format!("{}: {e}", source.display()))?;
		if kind.is_symlink() {
			let link = fs::read_link(&source).map_err(|e| format!("{}: {e}", source.display()))?;
			#[cfg(unix)]
			std::os::unix::fs::symlink(link, &target).map_err(|e| format!("{}: {e}", target.display()))?;
			#[cfg(not(unix))]
			return Err(format!("{}: a link to {}; package for macOS on a Mac", source.display(), link.display()));
		} else if kind.is_dir() {
			copy_dir(&source, &target)?;
		} else {
			fs::copy(&source, &target).map_err(|e| format!("{} -> {}: {e}", source.display(), target.display()))?;
		}
	}
	Ok(())
}

/// Whether a framework resource belongs in the package: every one but the locales other
/// than English (`<locale>.lproj`; the page is the game's own, as on Linux).
fn keep_resource(name: &str) -> bool {
	!name.ends_with(".lproj") || name == "en.lproj" || name == "Base.lproj"
}

/// Lay out `bb.app` in `dir` (emptied first) from `bb`, CEF's distribution and Steam's
/// library, and sign it ad hoc.
pub fn lay_out(dir: &Path, bb: &Path, cef_dir: &Path, steam: &Steam) -> Result<(), String> {
	let _ = fs::remove_dir_all(dir);
	let contents = dir.join(APP).join("Contents");
	let (macos, frameworks) = (contents.join("MacOS"), contents.join("Frameworks"));
	fs::create_dir_all(&macos).map_err(|e| format!("{}: {e}", macos.display()))?;
	fs::write(contents.join("Info.plist"), info_plist("bb", IDENTIFIER, false)).map_err(|e| e.to_string())?;
	let resources = contents.join("Resources");
	fs::create_dir_all(&resources).map_err(|e| format!("{}: {e}", resources.display()))?;
	fs::write(resources.join(crate::icns::FILE), crate::icns::game_icon()?).map_err(|e| e.to_string())?;
	let exe = macos.join("bb");
	crate::layout::copy(bb, &exe)?;
	if let Steam::Library(steam) = steam {
		crate::layout::copy(&steam.path, &macos.join(steam.pin.file_name))?;
		crate::steam_library::check_file(&macos.join(steam.pin.file_name), steam.pin.sha256)?;
	}

	let framework = frameworks.join(FRAMEWORK);
	copy_dir(&cef_dir.join(FRAMEWORK), &framework)?;
	let resources = framework.join("Resources");
	for entry in fs::read_dir(&resources).map_err(|e| format!("{}: {e}", resources.display()))?.flatten() {
		if !keep_resource(&entry.file_name().to_string_lossy()) {
			fs::remove_dir_all(entry.path()).map_err(|e| format!("{}: {e}", entry.path().display()))?;
		}
	}

	for (i, kind) in HELPER_KINDS.iter().enumerate() {
		let name = format!("{HELPER}{kind}");
		let helper = frameworks.join(format!("{name}.app")).join("Contents");
		fs::create_dir_all(helper.join("MacOS")).map_err(|e| format!("{}: {e}", helper.display()))?;
		let identifier = format!("{IDENTIFIER}.helper{}", if i == 0 { String::new() } else { format!(".{i}") });
		fs::write(helper.join("Info.plist"), info_plist(&name, &identifier, true)).map_err(|e| e.to_string())?;
		crate::layout::copy(&exe, &helper.join("MacOS").join(&name))?;
	}

	crate::run(std::process::Command::new("codesign").args(["--force", "--deep", "--sign", "-"]).arg(dir.join(APP)))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_helper_stays_out_of_the_dock() {
		let main = info_plist("bb", IDENTIFIER, false);
		let helper = info_plist(HELPER, "x.helper", true);
		assert!(main.contains("<key>CFBundleExecutable</key>\n\t<string>bb</string>"));
		assert!(!main.contains("LSUIElement"));
		assert!(main.contains("<key>CFBundleIconFile</key>\n\t<string>bb.icns</string>"));
		assert!(helper.contains("<key>LSUIElement</key>"));
		assert!(!helper.contains("CFBundleIconFile"));
		assert!(helper.contains("<string>bb Helper</string>"));
	}

	#[test]
	fn only_the_english_locale() {
		assert!(keep_resource("en.lproj") && keep_resource("icudtl.dat") && keep_resource("resources.pak"));
		assert!(!keep_resource("de.lproj") && !keep_resource("en_GB.lproj"));
	}
}
