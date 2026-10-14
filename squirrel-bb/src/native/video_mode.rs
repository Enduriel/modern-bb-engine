//! The video mode: the `VideoMode` script class and the mode the game runs in, shared
//! with the host ([`VideoSettings`]). As the original's (disassembled):
//! - `Settings.getVideoMode()` hands out the game's own mode, not a copy (`0x5008d0`
//!   returns the global, which its Sqrat wrapper `0x503860` pushes without copying):
//!   a script's writes to it change the mode at once. The resolution queries hand out
//!   copies.
//! - `Settings.setVideoMode(mode)` (`0x5008e0`) applies what the original applies at
//!   once: VSync off limits the frame rate to 144 (`LimitFrameRate`, the cap), VSync
//!   on lifts it; the depth of field, the UI scale and the scene scale (the cameras'
//!   closest zoom).
//! - The rest (size, colour bits, fullscreen, borderless) the original takes up only
//!   at its next start, from its config file. Here the host applies those at once too
//!   (an intentional divergence), and the mode then follows the window's real size.
//!
//! `Settings.save()` and `saveWithCustomVideoMode(mode)` ask the host to keep the mode
//! ([`VideoSettings::take_save`]).

use std::cell::RefCell;
use std::rc::Rc;

use squirrel::{ClassBuilder, Native, SQVM, SqClass, SqStrongRef};

use super::display_modes::{DisplayMode, Source};

/// The frame-rate cap with VSync off.
const FRAME_CAP_WITHOUT_VSYNC: i64 = 144;

/// A video mode: the original's fields (`VideoMode`, its Sqrat class at `0x4ffc80`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VideoMode {
	pub width: i64,
	pub height: i64,
	pub color_bits: i64,
	pub gamma: f64,
	pub brightness: f64,
	pub refresh_rate: i64,
	/// Frames per second at most, or 0 for no cap (not a script field).
	pub frame_cap: i64,
	pub ui_scale: f64,
	pub scene_scale: f64,
	pub fullscreen: bool,
	pub borderless: bool,
	pub vsync: bool,
	pub limit_frame_rate: bool,
	pub depth_of_field: bool,
}

impl VideoMode {
	/// The mode a headless engine starts in (the host replaces it at startup).
	pub const DEFAULT: VideoMode = VideoMode {
		width: 1920,
		height: 1080,
		color_bits: 32,
		gamma: 1.0,
		brightness: 1.0,
		refresh_rate: 60,
		frame_cap: 0,
		ui_scale: 1.0,
		scene_scale: 1.0,
		fullscreen: false,
		borderless: true,
		vsync: true,
		limit_frame_rate: false,
		depth_of_field: false,
	};

	/// The original's first start, without a config file, on a desktop of `width` ×
	/// `height` at `bits` (`0x49a39b`): fullscreen at the desktop's size, VSync on.
	pub fn first_start(width: i64, height: i64, bits: i64) -> VideoMode {
		VideoMode {
			width,
			height,
			color_bits: bits,
			fullscreen: true,
			borderless: false,
			..VideoMode::DEFAULT
		}
	}

	/// A display mode as the resolution queries hand it out.
	pub(crate) fn of(mode: DisplayMode) -> VideoMode {
		VideoMode {
			width: mode.width,
			height: mode.height,
			color_bits: mode.color_bits,
			..VideoMode::DEFAULT
		}
	}

	pub(crate) fn display_mode(&self) -> DisplayMode {
		DisplayMode {
			width: self.width,
			height: self.height,
			color_bits: self.color_bits,
		}
	}

	/// Set the frame-rate cap from VSync, as `setVideoMode` does (`0x5008e7`).
	pub fn cap_frame_rate(&mut self) {
		self.limit_frame_rate = !self.vsync;
		self.frame_cap = if self.vsync { 0 } else { FRAME_CAP_WITHOUT_VSYNC };
	}
}

struct VideoState {
	mode: VideoMode,
	/// How many times the scripts set a mode (`setVideoMode`).
	changes: u64,
	/// A mode the scripts asked to keep, until the host takes it.
	save: Option<VideoMode>,
	/// The monitor's resolutions, as the host reports them (`None` headless).
	monitor: Option<Vec<(i64, i64)>>,
}

/// The mode the game runs in, shared by the scripts' `Settings` and the host. The host
/// shows the UI at the mode's size divided by its UI scale, as the original does
/// (probed: at UIScale 1.5 on a 2560×1440 screen the page's viewport is 1706×960, at
/// device pixel ratio 1).
#[derive(Clone)]
pub struct VideoSettings(Rc<RefCell<VideoState>>);

impl Default for VideoSettings {
	fn default() -> Self {
		VideoSettings(Rc::new(RefCell::new(VideoState {
			mode: VideoMode::DEFAULT,
			changes: 0,
			save: None,
			monitor: None,
		})))
	}
}

impl VideoSettings {
	/// The mode now.
	pub fn mode(&self) -> VideoMode {
		self.0.borrow().mode
	}

	/// Set the mode, as the host starts it (from its config file) or applies it.
	pub fn set_mode(&self, mode: VideoMode) {
		self.0.borrow_mut().mode = mode;
	}

	/// How many times the scripts have set a mode: the host applies it when this moves.
	pub fn changes(&self) -> u64 {
		self.0.borrow().changes
	}

	/// Report the monitor's resolutions, which the Options screen lists.
	pub fn set_monitor_modes(&self, modes: Vec<(i64, i64)>) {
		self.0.borrow_mut().monitor = Some(modes);
	}

	/// What `f` lists the resolutions from (see `display_modes`).
	pub(crate) fn with_modes<R>(&self, f: impl FnOnce(Source) -> R) -> R {
		let s = self.0.borrow();
		f(Source {
			monitor: s.monitor.as_deref(),
			current: (s.mode.width, s.mode.height),
		})
	}

	/// The mode the scripts last asked to keep, once.
	pub fn take_save(&self) -> Option<VideoMode> {
		self.0.borrow_mut().save.take()
	}

	/// The screen's width and height, and the UI scale.
	pub fn screen(&self) -> (i64, i64, f64) {
		let mode = self.mode();
		(mode.width, mode.height, mode.ui_scale)
	}

	/// The scene's scale (`SceneScale`).
	pub fn scene_scale(&self) -> f64 {
		self.mode().scene_scale
	}

	/// The size the UI is laid out at: the screen divided by the UI scale, in whole
	/// pixels (cut down, as the original's 1706 for 2560 / 1.5).
	pub fn ui_size(&self) -> (i64, i64) {
		let (width, height, scale) = self.screen();
		let scale = if scale > 0.0 { scale } else { 1.0 };
		((width as f64 / scale) as i64, (height as f64 / scale) as i64)
	}

	/// Set the screen size, as the host's window has it.
	pub fn set_screen_size(&self, width: i64, height: i64) {
		let mut s = self.0.borrow_mut();
		s.mode.width = width;
		s.mode.height = height;
	}
}

/// The payload of a `VideoMode` instance: the game's own mode, or a copy.
#[derive(Clone)]
pub(crate) enum ModeRef {
	Live(VideoSettings),
	Copy(VideoMode),
}

impl ModeRef {
	pub(crate) fn get(&self) -> VideoMode {
		match self {
			ModeRef::Live(v) => v.mode(),
			ModeRef::Copy(m) => *m,
		}
	}

	fn update(&mut self, f: impl FnOnce(&mut VideoMode)) {
		match self {
			ModeRef::Live(v) => f(&mut v.0.borrow_mut().mode),
			ModeRef::Copy(m) => f(m),
		}
	}
}

/// Build the `VideoMode` class.
pub(crate) fn class(vm: &mut SQVM) -> SqClass<ModeRef> {
	macro_rules! field {
		($b:expr, $name:literal, $field:ident, $ty:ty) => {
			$b.property(
				$name,
				|v: &ModeRef| v.get().$field,
				|v: &mut ModeRef, x: $ty| v.update(|m| m.$field = x),
			)
		};
	}
	let mut b = ClassBuilder::<ModeRef>::new("VideoMode");
	b = field!(b, "Width", width, i64);
	b = field!(b, "Height", height, i64);
	b = field!(b, "ColorBits", color_bits, i64);
	b = field!(b, "Gamma", gamma, f64);
	b = field!(b, "Brightness", brightness, f64);
	b = field!(b, "RefreshRate", refresh_rate, i64);
	b = field!(b, "Fullscreen", fullscreen, bool);
	b = field!(b, "Borderless", borderless, bool);
	b = field!(b, "VSync", vsync, bool);
	b = field!(b, "LimitFrameRate", limit_frame_rate, bool);
	b = field!(b, "DepthOfField", depth_of_field, bool);
	b = field!(b, "UIScale", ui_scale, f64);
	b = field!(b, "SceneScale", scene_scale, f64);
	b.build(vm)
}

/// `getVideoMode`, `setVideoMode`, `save` and `saveWithCustomVideoMode`, over `current`.
pub(crate) fn natives(vm: &mut SQVM, class: &SqClass<ModeRef>, current: &VideoSettings) -> Vec<(&'static str, SqStrongRef)> {
	let get = {
		let (class, current) = (class.clone(), current.clone());
		vm.make_closure(move |vm: &SQVM| class.instantiate(vm, ModeRef::Live(current.clone())))
	};
	let set = {
		let current = current.clone();
		vm.make_closure(move |mut mode: Native<ModeRef>| {
			mode.0.update(VideoMode::cap_frame_rate);
			let applied = mode.0.get();
			let mut s = current.0.borrow_mut();
			s.mode = applied;
			s.changes += 1;
		})
	};
	let save = {
		let current = current.clone();
		vm.make_closure(move || {
			let mode = current.mode();
			current.0.borrow_mut().save = Some(mode);
		})
	};
	let save_with = {
		let current = current.clone();
		vm.make_closure(move |mode: Native<ModeRef>| current.0.borrow_mut().save = Some(mode.0.get()))
	};
	vec![("getVideoMode", get), ("setVideoMode", set), ("save", save), ("saveWithCustomVideoMode", save_with)]
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn vsync_sets_the_frame_cap() {
		let mut m = VideoMode { vsync: false, ..VideoMode::DEFAULT };
		m.cap_frame_rate();
		assert_eq!((m.limit_frame_rate, m.frame_cap), (true, 144));
		m.vsync = true;
		m.cap_frame_rate();
		assert_eq!((m.limit_frame_rate, m.frame_cap), (false, 0));
	}

	#[test]
	fn the_live_mode_is_shared() {
		let video = VideoSettings::default();
		let mut live = ModeRef::Live(video.clone());
		live.update(|m| m.ui_scale = 1.5);
		assert_eq!(video.mode().ui_scale, 1.5, "a script's write shows at once");
		let mut copy = ModeRef::Copy(video.mode());
		copy.update(|m| m.ui_scale = 2.0);
		assert_eq!(video.mode().ui_scale, 1.5, "a copy's doesn't");
		assert_eq!(video.ui_size(), (1280, 720));
	}
}
