//! How the game's video mode shows on the monitor. As the original's window code
//! (`0x5b5b20`): fullscreen takes the monitor in the mode's size (exclusive), borderless
//! is an undecorated window of the mode's size, and otherwise it's a window with a
//! title bar. Unlike the original's:
//! - A system without exclusive fullscreen (Wayland) gets borderless fullscreen, with
//!   the game's screen kept at the mode's size and scaled to the monitor, its aspect
//!   kept (as Proton shows the original there). So does a monitor without the size.
//! - A borderless mode of the monitor's size is borderless fullscreen, which the
//!   system then places over the whole monitor.
//! - A window can be resized, and the game's screen follows it (the original's window
//!   can't be), down to [`MIN_WINDOW`].

/// The smallest a window gets: the smallest resolution the Options screen lists
/// (vanilla's `Const.Options.Video.MinResolution`). The UI isn't laid out for less,
/// and the Options screen would list no resolution for a smaller window: its Apply then
/// fails as a whole ("Invalid index"), whatever else was changed.
pub const MIN_WINDOW: (u32, u32) = (1024, 768);

/// One of the monitor's modes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MonitorMode {
	pub width: u32,
	pub height: u32,
	pub bits: u16,
	pub refresh_millihertz: u32,
}

/// Where the game's screen goes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Placement {
	/// The monitor in its mode at this index.
	Exclusive(usize),
	/// Over the whole monitor, in its own mode.
	Borderless,
	/// A window, `decorated` with a title bar (and resizable) or not.
	Window { decorated: bool, width: u32, height: u32 },
}

impl Placement {
	/// Where `mode` goes on a monitor of `monitor_size` with `modes`, given whether the
	/// system has exclusive fullscreen.
	pub fn of(mode: &squirrel_bb::VideoMode, monitor_size: (u32, u32), modes: &[MonitorMode], exclusive: bool) -> Placement {
		let size = (mode.width.max(1) as u32, mode.height.max(1) as u32);
		if mode.fullscreen {
			let best = modes
				.iter()
				.enumerate()
				.filter(|(_, m)| (m.width, m.height) == size)
				.min_by_key(|(_, m)| {
					let bits = i64::from(m.bits) != mode.color_bits;
					let refresh = (i64::from(m.refresh_millihertz) - mode.refresh_rate * 1000).abs();
					(bits, refresh)
				});
			return match best {
				Some((i, _)) if exclusive => Placement::Exclusive(i),
				_ => Placement::Borderless,
			};
		}
		if mode.borderless && size == monitor_size {
			return Placement::Borderless;
		}
		Placement::Window {
			decorated: !mode.borderless,
			width: size.0.max(MIN_WINDOW.0),
			height: size.1.max(MIN_WINDOW.1),
		}
	}

	/// Whether the game's screen follows the window's size (a window), rather than
	/// keeping the mode's.
	pub fn follows_window(&self) -> bool {
		matches!(self, Placement::Window { .. })
	}
}

/// Where the frame goes in the window: `(x, y, width, height)`. A screen of the
/// window's size fills it; any other is scaled to fit, its aspect kept, and centred.
pub fn frame_rect(window: (u32, u32), screen: (u32, u32)) -> (u32, u32, u32, u32) {
	let (ww, wh) = (window.0.max(1), window.1.max(1));
	let (sw, sh) = (screen.0.max(1), screen.1.max(1));
	if u64::from(ww) * u64::from(sh) == u64::from(wh) * u64::from(sw) {
		return (0, 0, ww, wh);
	}
	let scale = (f64::from(ww) / f64::from(sw)).min(f64::from(wh) / f64::from(sh));
	let w = ((f64::from(sw) * scale).round() as u32).clamp(1, ww);
	let h = ((f64::from(sh) * scale).round() as u32).clamp(1, wh);
	((ww - w) / 2, (wh - h) / 2, w, h)
}

/// The present mode for VSync on or off, from those the surface offers: on, the
/// display's pace (FIFO, always offered); off, the newest frame without waiting
/// (mailbox, else immediate).
pub fn present_mode(vsync: bool, offered: &[wgpu::PresentMode]) -> wgpu::PresentMode {
	use wgpu::PresentMode::{Fifo, Immediate, Mailbox};
	if vsync {
		return Fifo;
	}
	[Mailbox, Immediate].into_iter().find(|m| offered.contains(m)).unwrap_or(Fifo)
}

#[cfg(test)]
mod tests {
	use super::*;
	use squirrel_bb::VideoMode;

	fn mode(width: u32, height: u32, refresh: u32) -> MonitorMode {
		MonitorMode {
			width,
			height,
			bits: 32,
			refresh_millihertz: refresh * 1000,
		}
	}

	#[test]
	fn fullscreen_takes_the_closest_mode_or_stands_in() {
		let modes = [mode(2560, 1440, 144), mode(1920, 1080, 144), mode(1920, 1080, 60), mode(1280, 720, 60)];
		let full = VideoMode::first_start(1920, 1080, 32);
		assert_eq!(Placement::of(&full, (2560, 1440), &modes, true), Placement::Exclusive(2), "60 Hz, as asked");
		assert_eq!(Placement::of(&full, (2560, 1440), &modes, false), Placement::Borderless, "Wayland");
		let odd = VideoMode::first_start(1700, 900, 32);
		assert_eq!(Placement::of(&odd, (2560, 1440), &modes, true), Placement::Borderless, "no such mode");
		assert!(!Placement::Borderless.follows_window());
	}

	#[test]
	fn borderless_and_windows() {
		let base = VideoMode {
			fullscreen: false,
			..VideoMode::first_start(2560, 1440, 32)
		};
		let borderless = VideoMode { borderless: true, ..base };
		assert_eq!(Placement::of(&borderless, (2560, 1440), &[], true), Placement::Borderless);
		let small = VideoMode {
			width: 1280,
			height: 800,
			..borderless
		};
		let window = Placement::of(&small, (2560, 1440), &[], true);
		assert_eq!(
			window,
			Placement::Window {
				decorated: false,
				width: 1280,
				height: 800
			}
		);
		assert!(window.follows_window());
		let decorated = Placement::of(&VideoMode { borderless: false, ..small }, (2560, 1440), &[], true);
		assert!(matches!(decorated, Placement::Window { decorated: true, .. }));
		let tiny = VideoMode {
			width: 800,
			height: 600,
			..small
		};
		assert_eq!(
			Placement::of(&tiny, (2560, 1440), &[], true),
			Placement::Window {
				decorated: false,
				width: 1024,
				height: 768
			},
			"no smaller than the Options screen's smallest"
		);
	}

	#[test]
	fn the_frame_keeps_its_aspect() {
		assert_eq!(frame_rect((2560, 1440), (2560, 1440)), (0, 0, 2560, 1440));
		assert_eq!(frame_rect((2560, 1440), (1280, 720)), (0, 0, 2560, 1440), "the same aspect fills");
		assert_eq!(frame_rect((2560, 1440), (1280, 1024)), (380, 0, 1800, 1440), "pillarboxed");
		assert_eq!(frame_rect((1280, 1024), (1920, 1080)), (0, 152, 1280, 720), "letterboxed");
	}

	#[test]
	fn vsync_picks_the_present_mode() {
		use wgpu::PresentMode::*;
		assert_eq!(present_mode(true, &[Fifo, Mailbox, Immediate]), Fifo);
		assert_eq!(present_mode(false, &[Fifo, Mailbox, Immediate]), Mailbox);
		assert_eq!(present_mode(false, &[Fifo, Immediate]), Immediate);
		assert_eq!(present_mode(false, &[Fifo]), Fifo);
	}
}
