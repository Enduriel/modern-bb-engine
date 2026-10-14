//! The screen the cameras project onto, with the original's pixel rules (its camera
//! base class, disassembled 2026-10-14: `screenToWorld` `0x4be5e0`, `worldToScreen`
//! `0x4be6a0`, `isInsideScreen` `0x4a3d00`, `screenToTile` `0x4ce080`). The camera's
//! position is the world point at the centre pixel, `(w / 2, h / 2)` rounded down;
//! screen y grows down where world y grows up; `zoom` is world units per pixel. Pixels
//! are whole and the arithmetic is `f32`, as there.

use squirrel::{Number, SQVM, SqStrongRef, SqTable};

/// The screen's size in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Screen {
	pub width: i32,
	pub height: i32,
}

impl Screen {
	/// The video mode (`Settings.getVideoMode()`), 1920x1080 if unreadable.
	pub(crate) fn of(vm: &SQVM) -> Screen {
		let mode = SqTable::root(vm)
			.get::<SqTable>("Settings")
			.and_then(|s| s.get::<SqStrongRef>("getVideoMode"))
			.and_then(|f| f.call_ret::<SqStrongRef, _>(vm, ()).ok().flatten());
		let dim = |name: &str| mode.as_ref().and_then(|m| m.get_member::<Number>(vm, name)).map(|n| n.0 as i32);
		Screen {
			width: dim("Width").unwrap_or(1920),
			height: dim("Height").unwrap_or(1080),
		}
	}

	/// The world point under pixel `(x, y)` with the camera at `pos`.
	pub(crate) fn to_world(&self, pos: (f32, f32), zoom: f32, x: i32, y: i32) -> (f32, f32) {
		let dx = x - self.width / 2;
		let dy = self.height - self.height / 2 - y;
		(dx as f32 * zoom + pos.0, dy as f32 * zoom + pos.1)
	}

	/// The pixel showing world point `world` with the camera at `pos`, truncated.
	pub(crate) fn to_screen(&self, pos: (f32, f32), zoom: f32, world: (f32, f32)) -> (i32, i32) {
		let x = (world.0 - pos.0) / zoom + (self.width / 2) as f32;
		let y = (world.1 - pos.1) / zoom + (self.height / 2) as f32;
		(x as i32, (self.height as f32 - y) as i32)
	}

	/// Whether `pixel` is on screen, at least `margin` pixels in from each edge.
	pub(crate) fn contains(&self, pixel: (i32, i32), margin: i32) -> bool {
		(margin..=self.width - margin).contains(&pixel.0) && (margin..=self.height - margin).contains(&pixel.1)
	}
}

#[cfg(test)]
mod tests {
	use super::Screen;

	const SCREEN: Screen = Screen {
		width: 2560,
		height: 1440,
	};

	#[test]
	fn the_centre_pixel_shows_the_camera_position() {
		assert_eq!(SCREEN.to_world((100.0, 200.0), 1.0, 1280, 720), (100.0, 200.0));
		assert_eq!(SCREEN.to_screen((100.0, 200.0), 1.0, (100.0, 200.0)), (1280, 720));
	}

	#[test]
	fn screen_y_grows_down_and_zoom_scales() {
		// One pixel right and one down: world x up, world y down, by the zoom.
		assert_eq!(SCREEN.to_world((0.0, 0.0), 2.0, 1281, 721), (2.0, -2.0));
		assert_eq!(SCREEN.to_screen((0.0, 0.0), 2.0, (2.0, -2.0)), (1281, 721));
	}

	#[test]
	fn an_odd_screen_rounds_its_centre_down() {
		let odd = Screen { width: 5, height: 5 };
		// The centre pixel is (2, 2); y counts 3 rows above it (5 - 2).
		assert_eq!(odd.to_world((0.0, 0.0), 1.0, 2, 2), (0.0, 1.0));
	}

	#[test]
	fn inside_means_within_the_margin() {
		assert!(SCREEN.contains((0, 0), 0));
		assert!(SCREEN.contains((2560, 1440), 0));
		assert!(!SCREEN.contains((-1, 10), 0));
		assert!(!SCREEN.contains((5, 10), 10));
	}
}
