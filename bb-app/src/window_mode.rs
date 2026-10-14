//! The game's video mode put on the window: the monitor it's on, and the window set to
//! a [`Placement`] (see `bb_app::display`).

use bb_app::display::{MIN_WINDOW, MonitorMode, Placement};
use winit::dpi::PhysicalSize;
use winit::event_loop::ActiveEventLoop;
use winit::monitor::{MonitorHandle, VideoModeHandle};
use winit::window::{Fullscreen, Window, WindowAttributes};

/// The size a game starts in without a monitor to go by.
const NO_MONITOR: (u32, u32) = (1920, 1080);

/// A monitor and its modes.
pub struct Monitor {
	handle: Option<MonitorHandle>,
	pub size: (u32, u32),
	pub modes: Vec<MonitorMode>,
	handles: Vec<VideoModeHandle>,
}

impl Monitor {
	/// The monitor a new window opens on: the primary one, or the first (Wayland names
	/// none primary).
	pub fn primary(event_loop: &ActiveEventLoop) -> Monitor {
		Monitor::of(event_loop.primary_monitor().or_else(|| event_loop.available_monitors().next()))
	}

	/// The monitor `window` is on, or as [`Monitor::primary`] when the system hasn't
	/// said (a Wayland surface that hasn't entered an output).
	pub fn of_window(window: &Window) -> Monitor {
		let handle = window.current_monitor();
		Monitor::of(handle.or_else(|| window.primary_monitor()).or_else(|| window.available_monitors().next()))
	}

	fn of(handle: Option<MonitorHandle>) -> Monitor {
		let handles: Vec<VideoModeHandle> = handle.iter().flat_map(|m| m.video_modes()).collect();
		let modes = handles
			.iter()
			.map(|m| MonitorMode {
				width: m.size().width,
				height: m.size().height,
				bits: m.bit_depth(),
				refresh_millihertz: m.refresh_rate_millihertz(),
			})
			.collect();
		let size = handle.as_ref().map(|m| m.size()).filter(|s| s.width > 0 && s.height > 0);
		Monitor {
			size: size.map_or(NO_MONITOR, |s| (s.width, s.height)),
			handle,
			modes,
			handles,
		}
	}

	/// Its resolutions, for the Options screen.
	pub fn resolutions(&self) -> Vec<(i64, i64)> {
		let mut sizes: Vec<(i64, i64)> = self
			.modes
			.iter()
			.map(|m| (i64::from(m.width), i64::from(m.height)))
			.chain([(i64::from(self.size.0), i64::from(self.size.1))])
			.collect();
		sizes.sort();
		sizes.dedup();
		sizes
	}

	fn fullscreen(&self, placement: Placement) -> Option<Fullscreen> {
		match placement {
			Placement::Exclusive(i) => self.handles.get(i).cloned().map(Fullscreen::Exclusive),
			Placement::Borderless => Some(Fullscreen::Borderless(self.handle.clone())),
			Placement::Window { .. } => None,
		}
	}
}

/// A new window's attributes for `placement` on `monitor`, for a screen of `size`. A
/// fullscreen window asks for the screen's size too, for a system that leaves its size
/// to it (gamescope does).
pub fn attributes(placement: Placement, monitor: &Monitor, size: (u32, u32)) -> WindowAttributes {
	let attributes = Window::default_attributes().with_title("Battle Brothers").with_resizable(true);
	match placement {
		Placement::Window { decorated, width, height } => attributes
			.with_decorations(decorated)
			.with_min_inner_size(PhysicalSize::new(MIN_WINDOW.0, MIN_WINDOW.1))
			.with_inner_size(PhysicalSize::new(width, height)),
		_ => attributes
			.with_inner_size(PhysicalSize::new(size.0, size.1))
			.with_fullscreen(monitor.fullscreen(placement)),
	}
}

/// Put `window` in `placement` on `monitor`, for a screen of `size`; the size it took
/// at once, if it did (the system reports it later otherwise, as a resize). Going
/// fullscreen, it asks for the screen's size first, as [`attributes`] does.
pub fn apply(window: &Window, placement: Placement, monitor: &Monitor, size: (u32, u32)) -> Option<PhysicalSize<u32>> {
	match placement {
		Placement::Window { decorated, width, height } => {
			window.set_fullscreen(None);
			window.set_decorations(decorated);
			window.set_resizable(true);
			window.set_min_inner_size(Some(PhysicalSize::new(MIN_WINDOW.0, MIN_WINDOW.1)));
			window.request_inner_size(PhysicalSize::new(width, height))
		}
		_ => {
			window.set_min_inner_size(None::<PhysicalSize<u32>>);
			let took = window.request_inner_size(PhysicalSize::new(size.0, size.1));
			window.set_fullscreen(monitor.fullscreen(placement));
			took
		}
	}
}
