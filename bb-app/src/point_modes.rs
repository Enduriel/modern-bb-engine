//! A Mac display's modes in points, for the Options screen's resolutions.
//!
//! The game's screen is in points on macOS (`screen_scale`), and fullscreen there is a
//! borderless window over the display in its present mode (`platform`): a mode only
//! sizes the game's screen. So the list is the sizes in points of the display's modes
//! (`CGDisplayModeGetWidth`/`Height`), the HiDPI ones included
//! (`kCGDisplayShowDuplicateLowResolutionModes`), each once, those the desktop can use,
//! and no larger than the display is now in points.
//!
//! winit's modes are the modes' pixels, and without the HiDPI ones: on the "Virtual
//! 16:9" display (5120x2880 pixels that look like 2560x1440) it lists 800x600 to
//! 2560x1440 at 1x and 5120x2880, and over the backing factor those became 400x300 to
//! 1280x720, below the Options screen's smallest (1024x768), and 2560x1440: the one
//! resolution the player saw.

/// A mode: its size in points, and whether the desktop can use it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PointMode {
	pub width: u32,
	pub height: u32,
	pub usable: bool,
}

/// The sizes to list from `modes` on a display now `screen` points: usable, no larger
/// than the screen either way, each once, widest first (taller first among equal
/// widths).
pub fn sizes(modes: &[PointMode], screen: (u32, u32)) -> Vec<(u32, u32)> {
	let mut sizes: Vec<(u32, u32)> = modes
		.iter()
		.filter(|m| m.usable && m.width > 0 && m.height > 0 && m.width <= screen.0 && m.height <= screen.1)
		.map(|m| (m.width, m.height))
		.collect();
	sizes.sort_by(|a, b| b.cmp(a));
	sizes.dedup();
	sizes
}

/// The modes of the display `display` (a `CGDirectDisplayID`), or `None` if it has
/// none to give.
#[cfg(target_os = "macos")]
pub fn of_display(display: u32) -> Option<Vec<PointMode>> {
	mac::modes(display)
}

#[cfg(target_os = "macos")]
mod mac {
	use std::ffi::c_void;

	use super::PointMode;

	type CFTypeRef = *const c_void;

	#[link(name = "CoreFoundation", kind = "framework")]
	unsafe extern "C" {
		static kCFBooleanTrue: CFTypeRef;
		static kCFTypeDictionaryKeyCallBacks: c_void;
		static kCFTypeDictionaryValueCallBacks: c_void;
		fn CFDictionaryCreate(
			allocator: CFTypeRef,
			keys: *const CFTypeRef,
			values: *const CFTypeRef,
			count: isize,
			key_callbacks: *const c_void,
			value_callbacks: *const c_void,
		) -> CFTypeRef;
		fn CFArrayGetCount(array: CFTypeRef) -> isize;
		fn CFArrayGetValueAtIndex(array: CFTypeRef, index: isize) -> CFTypeRef;
		fn CFRelease(object: CFTypeRef);
	}

	#[link(name = "CoreGraphics", kind = "framework")]
	unsafe extern "C" {
		static kCGDisplayShowDuplicateLowResolutionModes: CFTypeRef;
		fn CGDisplayCopyAllDisplayModes(display: u32, options: CFTypeRef) -> CFTypeRef;
		fn CGDisplayModeGetWidth(mode: CFTypeRef) -> usize;
		fn CGDisplayModeGetHeight(mode: CFTypeRef) -> usize;
		fn CGDisplayModeIsUsableForDesktopGUI(mode: CFTypeRef) -> bool;
	}

	pub(super) fn modes(display: u32) -> Option<Vec<PointMode>> {
		// SAFETY: CoreFoundation and CoreGraphics calls with the objects they made, each
		// released once; the array's values are borrowed while it lives.
		unsafe {
			let keys = [kCGDisplayShowDuplicateLowResolutionModes];
			let values = [kCFBooleanTrue];
			let options = CFDictionaryCreate(
				std::ptr::null(),
				keys.as_ptr(),
				values.as_ptr(),
				1,
				&raw const kCFTypeDictionaryKeyCallBacks,
				&raw const kCFTypeDictionaryValueCallBacks,
			);
			let array = CGDisplayCopyAllDisplayModes(display, options);
			if !options.is_null() {
				CFRelease(options);
			}
			if array.is_null() {
				return None;
			}
			let modes = (0..CFArrayGetCount(array))
				.map(|i| {
					let mode = CFArrayGetValueAtIndex(array, i);
					PointMode {
						width: u32::try_from(CGDisplayModeGetWidth(mode)).unwrap_or(0),
						height: u32::try_from(CGDisplayModeGetHeight(mode)).unwrap_or(0),
						usable: CGDisplayModeIsUsableForDesktopGUI(mode),
					}
				})
				.collect();
			CFRelease(array);
			Some(modes)
		}
	}

	#[cfg(test)]
	mod tests {
		/// The main display lists modes in points, its present size among them.
		#[test]
		fn the_main_display_has_modes_in_points() {
			unsafe extern "C" {
				fn CGMainDisplayID() -> u32;
			}
			// SAFETY: no arguments.
			let main = unsafe { CGMainDisplayID() };
			let modes = super::modes(main).expect("modes");
			assert!(modes.iter().any(|m| m.usable), "{modes:?}");
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn mode(width: u32, height: u32, usable: bool) -> PointMode {
		PointMode { width, height, usable }
	}

	/// The "Virtual 16:9" display at 2560x1440 points (HiDPI), as CoreGraphics lists it
	/// with the HiDPI modes.
	#[test]
	fn the_virtual_display_lists_its_sizes_in_points() {
		let modes = [
			mode(800, 600, true),
			mode(800, 600, true),
			mode(960, 540, true),
			mode(1024, 576, true),
			mode(1024, 768, true),
			mode(1280, 720, true),
			mode(1280, 960, true),
			mode(1344, 756, true),
			mode(1344, 1008, true),
			mode(1600, 900, true),
			mode(1600, 1200, true),
			mode(1920, 1080, true),
			mode(2560, 1440, true),
			mode(2560, 1440, true),
			mode(5120, 2880, true),
			mode(400, 300, false),
			mode(640, 480, false),
			mode(1280, 720, false),
		];
		assert_eq!(
			sizes(&modes, (2560, 1440)),
			[
				(2560, 1440),
				(1920, 1080),
				(1600, 1200),
				(1600, 900),
				(1344, 1008),
				(1344, 756),
				(1280, 960),
				(1280, 720),
				(1024, 768),
				(1024, 576),
				(960, 540),
				(800, 600),
			],
			"each once, none larger than the screen (5120x2880 at 1x)"
		);
		assert_eq!(sizes(&modes, (1600, 900)).first(), Some(&(1600, 900)), "the display at another mode");
		assert!(sizes(&[mode(1024, 768, false)], (2560, 1440)).is_empty());
	}
}
