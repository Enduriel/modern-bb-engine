//! How the original's cameras move (its camera class, `0x52dc10`, and its update,
//! `0x52dfd0`; disassembled 2026-10-14). All of it is `f32`, as there.
//!
//! - The position is the world point at the screen's centre; the zoom is world units
//!   per screen pixel, kept within `[min, max]`. The view covers the screen's size
//!   times the zoom, rounded to whole units.
//! - With bounds (the map's, set when the scene is sized), the view stays inside them:
//!   a move, a jump or each update pushes the position (and the target) back in. The
//!   furthest zoom out then also fits the map on the screen.
//! - A glide (`moveTo`, `moveToPos`) heads for a target at a speed in units per
//!   second; eased, it slows near the end (to `min(distance / speed + 0.15, 1)` of the
//!   speed). It lands on the target once within a unit, or stops where it is once
//!   within its arrival radius. A speed of 0 jumps.
//! - `zoomTo(zoom, speed)` and `zoomBy(delta)` (speed 8) ease the zoom: each update
//!   covers `delta x speed x dt` of what's left.
//! - The update runs once a frame, with the frame's real time.

/// A camera's state.
#[derive(Clone, Debug, PartialEq)]
pub struct Camera {
	pub pos: (f32, f32),
	target: (f32, f32),
	pub zoom: f32,
	zoom_target: f32,
	zoom_speed: f32,
	zoom_min: f32,
	/// The furthest the zoom goes: `zoom_limit`, or less to fit the bounds.
	zoom_max: f32,
	zoom_limit: f32,
	/// The screen's size in pixels.
	screen: (i32, i32),
	/// The view's size in world units.
	extent: (f32, f32),
	/// `(min x, min y, max x, max y)`.
	bounds: Option<[f32; 4]>,
	moving: bool,
	speed: f32,
	ease: bool,
	/// A glide stops where it is this close to its target (0: it lands on it).
	radius: f32,
}

/// The original's speed for `moveTo`, `moveToTile` and `moveToPos` without one.
pub const GLIDE_SPEED: f32 = 4096.0;

/// How fast `zoomBy` eases.
const ZOOM_BY_SPEED: f32 = 8.0;

/// Where an eased glide's speed bottoms out, as a share of its speed.
const EASE_FLOOR: f32 = 0.15;

/// `v` rounded to whole units, half away from zero.
fn round(v: f32) -> f32 {
	(if v < 0.0 { v - 0.5 } else { v + 0.5 }) as i32 as f32
}

impl Camera {
	/// A camera on a `screen`-sized screen, zooming out at most to `zoom_limit`.
	pub fn new(screen: (i32, i32), zoom_limit: f32) -> Camera {
		let mut c = Camera {
			pos: (0.0, 0.0),
			target: (0.0, 0.0),
			zoom: 1.0,
			zoom_target: 1.0,
			zoom_speed: ZOOM_BY_SPEED,
			zoom_min: 1.0,
			zoom_max: zoom_limit,
			zoom_limit,
			screen,
			extent: (0.0, 0.0),
			bounds: None,
			moving: false,
			speed: 1024.0,
			ease: false,
			radius: 0.0,
		};
		c.zoom_changed();
		c
	}

	/// The closest the zoom goes.
	pub fn set_zoom_min(&mut self, min: f32) {
		if min != self.zoom_min {
			self.zoom_min = min;
			self.zoom_changed();
		}
	}

	/// The screen's size changed.
	pub fn set_screen(&mut self, screen: (i32, i32)) {
		if screen != self.screen {
			self.screen = screen;
			self.fit_zoom();
			self.zoom_changed();
		}
	}

	/// Keep the view inside `min`..`max` (`0x4cdab0`, `0x4cdbe0`).
	pub fn set_bounds(&mut self, min: (f32, f32), max: (f32, f32)) {
		self.bounds = Some([min.0, min.1, max.0, max.1]);
		self.fit_zoom();
		self.zoom_changed();
		self.clamp();
	}

	/// `Zoom = v`: at once, within the limits (`0x4be240`).
	pub fn set_zoom(&mut self, v: f32) {
		self.zoom = v;
		self.zoom_target = v;
		self.zoom_changed();
	}

	/// `zoomTo(zoom, speed)`.
	pub fn zoom_to(&mut self, zoom: f32, speed: f32) {
		self.zoom_speed = speed;
		self.zoom_target = clamp_max_first(zoom, self.zoom_min, self.zoom_max);
	}

	/// `zoomBy(delta)`.
	pub fn zoom_by(&mut self, delta: f32) {
		self.zoom_speed = ZOOM_BY_SPEED;
		self.zoom_target = clamp_max_first(self.zoom_target + delta, self.zoom_min, self.zoom_max);
	}

	/// `setPos`: there at once.
	pub fn set_pos(&mut self, pos: (f32, f32)) {
		self.pos = pos;
		if self.bounds.is_some() {
			self.clamp();
		}
		self.target = self.pos;
	}

	/// `move(dx, dy)`: shifted, any glide ending where it is.
	pub fn shift(&mut self, d: (f32, f32)) {
		self.pos = (self.pos.0 + d.0, self.pos.1 + d.1);
		self.target = self.pos;
		self.radius = 0.0;
		if self.bounds.is_some() {
			self.clamp();
		}
	}

	/// `move(vec)`: shifted, a glide's target with it.
	pub fn shift_with_target(&mut self, d: (f32, f32)) {
		self.pos = (self.pos.0 + d.0, self.pos.1 + d.1);
		self.target = (self.target.0 + d.0, self.target.1 + d.1);
		self.radius = 0.0;
		if self.bounds.is_some() {
			self.clamp();
		}
	}

	/// Glide to `to` at `speed` (0: jump), eased or not, stopping within `radius`
	/// (`0x52e8c0`).
	pub fn glide(&mut self, to: (f32, f32), speed: f32, ease: bool, radius: f32) {
		self.radius = radius;
		if speed == 0.0 {
			self.pos = to;
			self.target = to;
		} else {
			self.speed = speed;
			self.target = to;
			self.ease = ease;
			self.moving = true;
		}
		if self.bounds.is_some() {
			self.clamp();
		}
	}

	/// `cancelMovement`: the glide ends where the camera is.
	pub fn cancel(&mut self) {
		self.target = self.pos;
		self.moving = false;
		self.radius = 0.0;
	}

	/// Where a glide is heading (the position when not gliding).
	pub fn target(&self) -> (f32, f32) {
		self.target
	}

	/// One frame, `dt` seconds of real time (`0x52dfd0`).
	pub fn update(&mut self, dt: f32) {
		if self.zoom_target != self.zoom {
			let left = self.zoom_target - self.zoom;
			let step = left * dt * self.zoom_speed;
			let step = if left < 0.0 {
				if left <= step { step } else { left }
			} else if step <= left {
				step
			} else {
				left
			};
			self.zoom += step;
			self.zoom_changed();
		}
		if self.moving {
			self.glide_step(dt);
		}
		if self.bounds.is_some() {
			self.clamp();
		}
	}

	fn glide_step(&mut self, dt: f32) {
		let (dx, dy) = (self.pos.0 - self.target.0, self.pos.1 - self.target.1);
		let distance = f64::from(dy * dy + dx * dx).sqrt() as f32;
		let near = if self.radius > 1.0 { self.radius } else { 1.0 };
		if near > distance {
			if self.radius == 0.0 {
				self.pos = self.target;
			} else {
				self.target = self.pos;
			}
			self.moving = false;
			self.radius = 0.0;
			return;
		}
		let (dx, dy) = (self.target.0 - self.pos.0, self.target.1 - self.pos.1);
		let inv = 1.0 / f64::from(dy * dy + dx * dx).sqrt() as f32;
		let dir = (inv * dx, inv * dy);
		let pace = if self.ease {
			let left = f64::from(dy * dy + dx * dx).sqrt() as f32;
			(left / self.speed + EASE_FLOOR).min(1.0)
		} else {
			1.0
		};
		let x = dir.0 * dt * self.speed * pace + self.pos.0;
		self.pos.0 = if dir.0 > 0.0 { x.min(self.target.0) } else { x.max(self.target.0) };
		let y = dir.1 * dt * self.speed * pace + self.pos.1;
		self.pos.1 = if dir.1 > 0.0 { y.min(self.target.1) } else { y.max(self.target.1) };
	}

	/// The zoom kept within its limits, and the view's size with it (`0x52e540`).
	fn zoom_changed(&mut self) {
		self.zoom = clamp_max_first(self.zoom, self.zoom_min, self.zoom_max);
		self.extent = (round(self.screen.0 as f32 * self.zoom), round(self.screen.1 as f32 * self.zoom));
	}

	/// The furthest zoom: the limit, or what shows all of the bounds (`0x52ec10`).
	fn fit_zoom(&mut self) {
		self.zoom_max = match self.bounds {
			None => self.zoom_limit,
			Some([x0, y0, x1, y1]) => {
				let x = ((x1 - x0) / self.screen.0 as f32).min(self.zoom_limit);
				let y = ((y1 - y0) / self.screen.1 as f32).min(self.zoom_limit);
				if y <= x { y } else { x }
			}
		};
	}

	/// The position and target pushed back inside the bounds (`0x52eb10`).
	fn clamp(&mut self) {
		let Some([x0, y0, x1, y1]) = self.bounds else { return };
		let (hw, hh) = (self.extent.0 * 0.5, self.extent.1 * 0.5);
		let inside = |v: f32, half: f32, lo: f32, hi: f32| {
			let v = if v + half > hi { hi - half } else { v };
			if lo > v - half { lo + half } else { v }
		};
		self.pos = (inside(self.pos.0, hw, x0, x1), inside(self.pos.1, hh, y0, y1));
		self.target = (inside(self.target.0, hw, x0, x1), inside(self.target.1, hh, y0, y1));
	}
}

/// `v` within `[min, max]`, the maximum checked first (as the original's clamps are).
fn clamp_max_first(v: f32, min: f32, max: f32) -> f32 {
	if v > max {
		max
	} else if min > v {
		min
	} else {
		v
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn camera() -> Camera {
		Camera::new((1000, 500), 12.0)
	}

	#[test]
	fn a_plain_glide_goes_at_its_speed_and_lands() {
		let mut c = camera();
		c.glide((300.0, 400.0), 100.0, false, 0.0);
		c.update(1.0);
		let close = |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3;
		assert!(close(c.pos, (60.0, 80.0)), "100 units along (3, 4): {:?}", c.pos);
		assert!(c.moving);
		for _ in 0..4 {
			c.update(1.0);
		}
		assert_eq!(c.pos, (300.0, 400.0), "never past the target");
		c.update(0.0);
		assert!(!c.moving, "within a unit, it lands");
	}

	#[test]
	fn an_eased_glide_slows_near_the_end() {
		let mut c = camera();
		c.glide((1000.0, 0.0), 4096.0, true, 0.0);
		c.update(0.01);
		// (1000 / 4096 + 0.15) of 4096 per second.
		assert!((c.pos.0 - (1000.0 / 4096.0 + 0.15) * 40.96).abs() < 1e-3, "{:?}", c.pos);
		let mut far = camera();
		far.glide((10000.0, 0.0), 4096.0, true, 0.0);
		far.update(0.01);
		assert!((far.pos.0 - 40.96).abs() < 1e-3, "full speed far away: {:?}", far.pos);
	}

	#[test]
	fn a_glide_with_a_radius_stops_where_it_is() {
		let mut c = camera();
		c.glide((100.0, 0.0), 95.0, false, 10.0);
		c.update(1.0);
		c.update(0.0);
		assert_eq!(c.pos, (95.0, 0.0));
		assert_eq!(c.target(), (95.0, 0.0));
		assert!(!c.moving);
	}

	#[test]
	fn speed_zero_jumps() {
		let mut c = camera();
		c.glide((50.0, 60.0), 0.0, false, 0.0);
		assert_eq!(c.pos, (50.0, 60.0));
		assert!(!c.moving);
	}

	#[test]
	fn the_zoom_eases_and_keeps_its_limits() {
		let mut c = camera();
		c.zoom_to(2.0, 4.0);
		c.update(0.1);
		assert!((c.zoom - 1.4).abs() < 1e-6, "{}", c.zoom);
		c.update(1.0);
		assert_eq!(c.zoom, 2.0, "never past the target");
		c.set_zoom(40.0);
		assert_eq!(c.zoom, 12.0);
		c.set_zoom(0.1);
		assert_eq!(c.zoom, 1.0);
	}

	#[test]
	fn bounds_keep_the_view_inside_and_cap_the_zoom() {
		let mut c = camera();
		c.set_bounds((0.0, 0.0), (4000.0, 3000.0));
		c.set_pos((0.0, 0.0));
		assert_eq!(c.pos, (500.0, 250.0), "the view's corner on the bounds'");
		c.set_zoom(12.0);
		assert_eq!(c.zoom, 4.0, "zoomed out no further than the map is wide");
		c.set_pos((10000.0, 10000.0));
		assert_eq!(c.pos, (2000.0, 2000.0), "a 4000 x 2000 view");
	}
}
