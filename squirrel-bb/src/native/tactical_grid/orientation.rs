//! Where the battle camera shows a unit's orientation overlay (its bridge's overlay
//! function, `0x4bf4e0`; disassembled 2026-10-20), which `updateEntityOverlays` and
//! `queryEntityOverlays` run for each overlay (see `camera_overlays` for the sprite).
//!
//! - On a tile the player doesn't see now: never, reported offscreen at `(0, 0)`.
//! - With the `ShowOrientationOverlays` option on (and a gliding camera: an instant one
//!   shows everything, see `camera`), a unit whose line to the screen's
//!   centre (the camera's position, rounded) crosses the edge of the view shows where
//!   it crosses: the view's rectangle in world units, kept inside the map's bounds,
//!   edges in the order top, right, bottom, left.
//! - Otherwise (on screen, or the option off) over its tile, drawn lower by 40 pixels
//!   (at zoom 1) for each level its tile is above the camera's. It shows when its tile
//!   is above the camera's level (nothing on such a tile is drawn), or when the tile in
//!   front of it is two or more levels higher and the camera is more than a level away
//!   from the unit's (`entity.isOccludedByTerrain()`, `0x4c3030`).
//!
//! All of it in `f32`, positions truncated to whole pixels, as there. The camera's
//! `render()` draws them only with the option on (`0x4a4046`).

use squirrel::{SQVM, SqArray, SqStrongRef, SqTable};

use super::{Grid, visibility};
use crate::native::Sprites;
use crate::native::camera_overlays::Located;
use crate::native::screen::Screen;
use crate::native::sprite_color::real_time;

/// Screen pixels per level of a tile above the camera's (`0x63d36c`).
const LEVEL_PIXELS: f32 = 40.0;

/// What the camera's bridge reads of the camera.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct View {
	/// The world point at the screen's centre.
	pub pos: (f32, f32),
	/// World units per pixel.
	pub zoom: f32,
	/// The screen, in pixels.
	pub screen: (i32, i32),
	/// The view, in world units (the screen's size times the zoom, rounded).
	pub extent: (f32, f32),
	/// What the view may show: `[min x, min y, max x, max y]`.
	pub bounds: [f32; 4],
	pub level: i64,
	/// The `ShowOrientationOverlays` option, and a camera that can leave units off
	/// screen: an instant one shows everything (see `camera`), so none is ever placed
	/// on an edge.
	pub offscreen_overlays: bool,
}

/// What the bridge reads of the overlaid unit's tile.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Unit {
	/// The tile's world position (its level's height included).
	pub pos: (f32, f32),
	pub level: i64,
	/// The player sees the tile now.
	pub seen: bool,
	/// The tile in front is two or more levels higher (`isOccludedByTerrain`).
	pub occluded: bool,
}

/// `v` rounded to a whole number, half away from zero.
fn round(v: f32) -> f32 {
	(if v < 0.0 { v - 0.5 } else { v + 0.5 }) as i32 as f32
}

impl View {
	/// The view's rectangle, kept inside the bounds: `(left, bottom, right, top)`
	/// (`0x4a3b60`).
	fn rect(&self) -> (f32, f32, f32, f32) {
		let [x0, y0, x1, y1] = self.bounds;
		let (hw, hh) = (self.extent.0 * 0.5, self.extent.1 * 0.5);
		let min = |v: f32, bound: f32| if bound > v { bound } else { v };
		let max = |v: f32, bound: f32| if v > bound { bound } else { v };
		(
			min(self.pos.0 - hw, x0),
			min(self.pos.1 - hh, y0),
			max(self.pos.0 + hw, x1),
			max(self.pos.1 + hh, y1),
		)
	}

	/// The pixel showing world point `p`, from the top left, truncated.
	fn pixel(&self, p: (f32, f32)) -> (i32, i32) {
		let x = (p.0 - self.pos.0) / self.zoom + (self.screen.0 / 2) as f32;
		let y = (p.1 - self.pos.1) / self.zoom + (self.screen.1 / 2) as f32;
		(x as i32, (self.screen.1 as f32 - y) as i32)
	}

	/// Where the line from `p` to the screen's centre leaves the view, if it does.
	fn exit(&self, p: (f32, f32)) -> Option<(f32, f32)> {
		let d = (round(self.pos.0) - p.0, round(self.pos.1) - p.1);
		let (left, bottom, right, top) = self.rect();
		let corners = [(left, top), (right, top), (right, bottom), (left, bottom), (left, top)];
		corners.windows(2).find_map(|edge| {
			let (a, b) = (edge[0], edge[1]);
			let e = (b.0 - a.0, b.1 - a.1);
			let denom = e.1 * d.0 - e.0 * d.1;
			if denom == 0.0 {
				return None;
			}
			let r = (p.0 - a.0, p.1 - a.1);
			let t = (r.1 * e.0 - r.0 * e.1) / denom;
			let u = (r.1 * d.0 - r.0 * d.1) / denom;
			let within = |v: f32| (0.0..=1.0).contains(&v);
			(within(t) && within(u)).then(|| (t * d.0 + p.0, t * d.1 + p.1))
		})
	}

	/// Where `unit`'s overlay goes (before it is kept on screen; see
	/// `EntityOverlays::place`).
	pub(crate) fn locate(&self, unit: Unit) -> Located {
		if !unit.seen {
			return Located {
				offscreen: true,
				..Located::default()
			};
		}
		if self.offscreen_overlays && let Some(edge) = self.exit(unit.pos) {
			let (x, y) = self.pixel(edge);
			return Located {
				x,
				y,
				visible: true,
				offscreen: true,
				y_offset: 0,
			};
		}
		let (x, y) = self.pixel(unit.pos);
		let above = unit.level - self.level;
		let visible = above > 0 || (unit.occluded && above.abs() > 1);
		Located {
			x,
			y,
			visible,
			offscreen: false,
			y_offset: (above.max(0) as f32 * LEVEL_PIXELS / self.zoom) as i32,
		}
	}
}

/// The `ShowOrientationOverlays` option (on unless the settings say otherwise).
fn option_on(vm: &SQVM) -> bool {
	crate::native::gameplay_flag(vm, "ShowOrientationOverlays", true)
}

/// An overlay image's size in pixels, `None` for a brush that isn't loaded. Before
/// the brushes are loaded every name is taken, at no size.
pub(super) fn brush_size(sprites: &Sprites, brush: &str) -> Option<(i32, i32)> {
	if sprites.resolve_battle(brush).is_empty() {
		return None;
	}
	Some(sprites.with(|r| r.get(brush).map_or((0, 0), |(_, s)| (s.width, s.height))))
}

/// `updateEntityOverlays()` (`0x4a40c0`), or with `query`, `queryEntityOverlays()`
/// (`0x4a4110`): each overlay placed where [`View::locate`] has it, at the real time
/// now. The update skips units not on the map; the query lists every overlay, one not
/// on the map as an empty table (the bridge returns before filling it in).
pub(super) fn update(grid: &Grid, vm: &SQVM, query: bool) -> Option<SqStrongRef> {
	let now = real_time(vm);
	let screen = Screen::of(vm);
	let screen = (screen.width, screen.height);
	let option = option_on(vm);
	let lookup = grid.borrow().unit_tile_lookup.clone();
	let mut list = query.then(|| SqArray::new(vm));
	let count = grid.borrow().camera.overlays.list().len();
	for i in 0..count {
		let identity = grid.borrow().camera.overlays.list()[i].identity;
		let tile = lookup.as_ref().and_then(|l| l(identity));
		let mut g = grid.borrow_mut();
		let Some(unit) = tile.and_then(|t| g.overlay_unit(t)) else {
			if let Some(list) = list.as_mut() {
				list.push(SqTable::new(vm).to_strong());
			}
			continue;
		};
		let view = g.camera.overlay_view(screen, (g.width, g.height), option);
		let at = view.locate(unit);
		let pos = g.camera.overlays.place(i, at, screen, now);
		if let Some(list) = list.as_mut() {
			list.push(g.camera.overlays.state(vm, i, at, pos));
		}
	}
	list.map(|l| l.to_strong())
}

/// The camera's `render()` (`0x4a4030`): with a battle map and the option on, the
/// overlays shown are drawn (see `EntityOverlays::render`).
pub(super) fn render(grid: &Grid, vm: &SQVM) {
	if !option_on(vm) {
		return;
	}
	let now = real_time(vm);
	let mut g = grid.borrow_mut();
	if g.width > 0 && g.height > 0 {
		g.camera.overlays.render(now);
	}
}

impl super::TacticalGridState {
	/// What the overlays' placing reads of the tile a unit stands on.
	fn overlay_unit(&self, tile: usize) -> Option<Unit> {
		let t = self.tiles.get(tile)?;
		let pos = t.world_pos();
		Some(Unit {
			pos: (pos.0 as f32, pos.1 as f32),
			level: t.level,
			seen: t.vis_mask.contains(visibility::PLAYER),
			occluded: self.occluded_by_terrain(tile),
		})
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A view probed in the original: camera at (1700, 1310), zoom 1, 2560x1440, on a
	/// map whose bounds don't reach into the view.
	fn view(level: i64) -> View {
		View {
			pos: (1700.0, 1310.0),
			zoom: 1.0,
			screen: (2560, 1440),
			extent: (2560.0, 1440.0),
			bounds: [-1000.0, -1000.0, 9000.0, 9000.0],
			level,
			offscreen_overlays: true,
		}
	}

	fn unit(pos: (f32, f32), level: i64) -> Unit {
		Unit {
			pos,
			level,
			seen: true,
			occluded: false,
		}
	}

	#[test]
	fn a_unit_above_the_cameras_level_shows_over_its_tile_lowered_by_the_levels() {
		// The probe's B: tile (11, 15) at level 1, camera at level 0. The original drew
		// its portrait centred on (980, 750).
		let at = view(0).locate(unit((1400.0, 1320.0), 1));
		assert_eq!((at.x, at.y, at.y_offset), (980, 710, 40));
		assert!(at.visible && !at.offscreen);
		// The camera at its level or above: nothing to show.
		assert!(!view(1).locate(unit((1400.0, 1320.0), 1)).visible);
	}

	#[test]
	fn a_unit_hidden_by_the_tile_in_front_shows_unless_the_camera_is_within_a_level() {
		let hidden = Unit {
			occluded: true,
			..unit((1400.0, 1280.0), 0)
		};
		assert!(view(2).locate(hidden).visible);
		assert!(!view(1).locate(hidden).visible);
		assert_eq!(view(2).locate(hidden).y_offset, 0);
	}

	#[test]
	fn a_unit_off_screen_shows_where_its_line_to_the_centre_leaves_the_view() {
		// Straight right of the centre, past the right edge (x 2980): on that edge.
		let at = view(3).locate(unit((4000.0, 1310.0), 0));
		assert!(at.visible && at.offscreen);
		assert_eq!((at.x, at.y), (2560, 720));
		// Off screen, but the option off: where it is, not shown.
		let off = View {
			offscreen_overlays: false,
			..view(3)
		};
		let at = off.locate(unit((4000.0, 1310.0), 0));
		assert!(!at.visible && !at.offscreen);
		assert_eq!((at.x, at.y), (3580, 720));
	}

	#[test]
	fn a_unit_the_player_does_not_see_shows_nothing() {
		let at = view(0).locate(Unit {
			seen: false,
			..unit((1400.0, 1320.0), 1)
		});
		assert_eq!(at, Located {
			offscreen: true,
			..Located::default()
		});
	}

	#[test]
	fn the_view_is_cut_to_the_maps_bounds() {
		// Bounds ending at x 2000: a unit at x 2500 is past them, on screen or not.
		let v = View {
			bounds: [0.0, 0.0, 2000.0, 9000.0],
			..view(3)
		};
		let at = v.locate(unit((2500.0, 1310.0), 0));
		assert!(at.offscreen);
		assert_eq!((at.x, at.y), (1580, 720));
	}
}
