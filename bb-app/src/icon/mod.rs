//! The game's icon on bb's windows (the game's and the not-found one), from the
//! original's own `.ico` (`assets/bb.ico`; its README says where it comes from):
//! - Windows: `bb.exe` carries it as its resource ([`res`], linked in by `build.rs`), for
//!   Explorer and the taskbar; the windows load it from there, each size as the system
//!   wants it.
//! - X11: the window's `_NET_WM_ICON`, through winit.
//! - Wayland: winit sets none there; the window's toplevel gets it through the
//!   compositor's `xdg_toplevel_icon_manager_v1`, where it has one ([`wayland`]; KDE's
//!   does, GNOME's doesn't: the desktop's default icon shows there).
//! - macOS: the app bundle's (`bb-package` writes its `.icns` from the same file). A
//!   window has no icon of its own there.

pub mod ico;
#[cfg(any(test, windows))]
pub mod res;
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
mod wayland;

use winit::event_loop::ActiveEventLoop;
use winit::window::{Icon, Window, WindowAttributes};

/// The original's icon file (16, 32 and 48 pixels).
pub const ICO: &[u8] = include_bytes!("../../assets/bb.ico");

/// The icon's images, smallest first.
pub fn images() -> Vec<ico::Image<'static>> {
	let mut images = ico::images(ICO).expect("assets/bb.ico is a valid icon");
	images.sort_by_key(|i| i.size);
	images
}

/// `attributes` with the icon, where the system takes it with the window's.
pub fn with_icon(attributes: WindowAttributes) -> WindowAttributes {
	#[cfg(windows)]
	{
		use winit::platform::windows::WindowAttributesExtWindows;
		// The title bar's (small) and the taskbar's and Alt+Tab's (big) icons.
		attributes.with_window_icon(window_icon()).with_taskbar_icon(window_icon())
	}
	#[cfg(not(windows))]
	attributes.with_window_icon(window_icon())
}

/// The icon for a window: on Windows from `bb.exe`'s resource, the image at the
/// system's icon size; elsewhere, or in a program without the resource (a test's), the
/// largest image.
pub fn window_icon() -> Option<Icon> {
	#[cfg(windows)]
	{
		use winit::platform::windows::IconExtWindows;
		if let Ok(icon) = Icon::from_resource(res::GROUP_ID, None) {
			return Some(icon);
		}
	}
	let largest = images().pop()?;
	Icon::from_rgba(largest.rgba().ok()?, largest.size, largest.size).ok()
}

/// What keeps the icon on a window that needs more than its attributes (Wayland's):
/// kept as long as the window.
pub struct Shown {
	#[cfg(any(target_os = "linux", target_os = "freebsd"))]
	_wayland: Option<wayland::ToplevelIcon>,
}

/// Put the icon on `window`, where its attributes didn't (Wayland, when the
/// compositor offers the protocol: silently nothing otherwise).
pub fn show(window: &Window, event_loop: &ActiveEventLoop) -> Shown {
	#[cfg(any(target_os = "linux", target_os = "freebsd"))]
	return Shown { _wayland: wayland::ToplevelIcon::set(window, &event_loop.owned_display_handle()) };
	#[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
	{
		let _ = (window, event_loop);
		Shown {}
	}
}

#[cfg(test)]
mod tests;
