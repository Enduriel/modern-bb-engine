//! The camera class's entity overlays, shared by the world's camera and the battle's:
//! `addEntityOverlay({ entity, image })`, `removeEntityOverlay(entity)`,
//! `removeEntityOverlays()` and `queryEntityOverlays()` (vanilla's battle orientation
//! overlays, orientation_overlay.nut; disassembled 2026-10-20, the camera class's
//! `0x4a40c0`…`0x4a4590`).
//!
//! Each overlay is a sprite of its `image` brush the camera keeps for the unit, drawn in
//! screen space by the camera's `render()` (`0x4a4030`), not by the UI: the page's
//! `.overlay` divs are invisible click targets (`rgba(0, 0, 0, 0.01)`) placed from what
//! `queryEntityOverlays` reports. The sprite starts hidden, white and opaque; an image
//! whose name contains `banner` (a player's, the company banner) is drawn at 3/4 size.
//! Where the camera's bridge decides an overlay shows (the battle's: see
//! `tactical_grid::orientation`), [`EntityOverlays::place`] fades it in or out over
//! 200 ms of real time (`0x4a4910`) and keeps it on screen.

use squirrel::{SQVM, SqArray, SqStrongRef, SqTable};

use crate::native::sprite_color::SpriteColor;
use crate::scene::tactical::SceneOrientationOverlay;

/// How long an overlay fades in or out, in milliseconds (`0x4a4965`).
const FADE_MS: i64 = 200;
/// How far past a screen edge an overlay may reach, in pixels (`0x4a4a12`).
const EDGE_MARGIN: i32 = 10;
/// A `banner` image's scale (`0x4a47aa`).
const BANNER_SCALE: f32 = 0.75;

/// One overlay: the unit it follows and its sprite.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Overlay {
	/// The unit's `getID()`.
	pub id: i64,
	/// The unit's script identity (how the host looks it up).
	pub identity: i64,
	pub brush: String,
	/// The brush's picture size, in pixels (`+0x48`, `+0x4c`), which keeps it on screen.
	size: (i32, i32),
	pub scale: f32,
	pub color: SpriteColor,
	/// The sprite is drawn (its scene flag bit 0).
	pub visible: bool,
	/// The overlay last faded in, rather than out (`+0x18` of its record).
	shown: bool,
	/// Where it is drawn: screen pixels from the centre, y up.
	pub pos: (f32, f32),
}

/// An overlaid unit's place on screen, as the camera's bridge works it out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Located {
	/// Pixels from the top left: the unit's tile, or where the line to it leaves the
	/// screen.
	pub x: i32,
	pub y: i32,
	/// The overlay should show.
	pub visible: bool,
	pub offscreen: bool,
	/// Pixels to draw it lower by: its tile's levels above the camera's.
	pub y_offset: i32,
}

/// The overlays, in the order they were added, and what the last `render()` drew.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct EntityOverlays {
	list: Vec<Overlay>,
	drawn: Vec<SceneOrientationOverlay>,
}

/// An entity's id (`getID()`).
fn entity_id(vm: &SQVM, entity: &SqStrongRef) -> Option<i64> {
	entity.call_method_ret::<i64, _>(vm, "getID", ()).ok().flatten()
}

impl EntityOverlays {
	/// `addEntityOverlay({ entity, image })`, once per entity: `brush_size` gives the
	/// image brush's size, `None` for an unknown brush, which adds nothing (the original
	/// logs that it failed). An empty image adds nothing either.
	pub(crate) fn add(&mut self, vm: &SQVM, overlay: &SqStrongRef, brush_size: impl Fn(&str) -> Option<(i32, i32)>) {
		let Some(table) = overlay.as_table(vm) else { return };
		let (Some(entity), Some(image)) = (table.get::<SqStrongRef>("entity"), table.get::<String>("image")) else {
			return;
		};
		let Some(id) = entity_id(vm, &entity) else { return };
		if image.is_empty() || self.list.iter().any(|o| o.id == id) {
			return;
		}
		let Some(size) = brush_size(&image) else { return };
		self.list.push(Overlay {
			id,
			identity: crate::native::tactical_entities::unit_identity(vm, &entity),
			scale: if image.contains("banner") { BANNER_SCALE } else { 1.0 },
			brush: image,
			size,
			color: SpriteColor::default(),
			visible: false,
			shown: false,
			pos: (0.0, 0.0),
		});
	}

	/// `removeEntityOverlay(entity)`.
	pub(crate) fn remove(&mut self, vm: &SQVM, entity: &SqStrongRef) {
		if let Some(id) = entity_id(vm, entity) {
			self.list.retain(|o| o.id != id);
		}
	}

	/// `removeEntityOverlays()`.
	pub(crate) fn clear(&mut self) {
		self.list.clear();
		self.drawn.clear();
	}

	/// The overlays.
	pub(crate) fn list(&self) -> &[Overlay] {
		&self.list
	}

	/// Overlay `i` shows as `at` says (`0x4a4910`): fading in if it wasn't shown, out
	/// (hiding at the end) if it was, at real time `now`. A shown overlay is kept within
	/// 10 pixels past each edge of a `screen`-sized screen and drawn `y_offset` lower;
	/// returns where it is then (pixels from the top left), or `(0, 0)` for one that
	/// doesn't show.
	pub(crate) fn place(&mut self, i: usize, at: Located, screen: (i32, i32), now: f32) -> (i32, i32) {
		let Some(o) = self.list.get_mut(i) else { return (0, 0) };
		if !at.visible {
			if o.shown {
				o.color.fade_out(FADE_MS, now, true);
				o.shown = false;
			}
			return (0, 0);
		}
		if !o.shown {
			o.visible = true;
			o.color.fade_in(FADE_MS, now);
			o.shown = true;
		}
		let (half_w, half_h) = (o.size.0 / 2, o.size.1 / 2);
		let keep = |v: i32, half: i32, extent: i32| {
			let v = if v - half <= -EDGE_MARGIN { half - EDGE_MARGIN } else { v };
			if v + half >= extent + EDGE_MARGIN { extent - half + EDGE_MARGIN } else { v }
		};
		let (x, y) = (keep(at.x, half_w, screen.0), keep(at.y, half_h, screen.1));
		let (w, h) = (screen.0 as f32, screen.1 as f32);
		o.pos = (x as f32 - w * 0.5, h - (at.y_offset + y) as f32 - h * 0.5);
		(x, y)
	}

	/// `queryEntityOverlays()`'s entry for overlay `i` (`0x4bfa7a`): `{ id, visible,
	/// isOffscreen, yOffset, x, y }`, the UI's fields
	/// (`TacticalScreenIdentifier.OrientationOverlay`).
	pub(crate) fn state(&self, vm: &SQVM, i: usize, at: Located, (x, y): (i32, i32)) -> SqStrongRef {
		let mut t = SqTable::new(vm);
		if let Some(o) = self.list.get(i) {
			t.set("id", o.id);
			t.set("visible", at.visible);
			t.set("isOffscreen", at.offscreen);
			t.set("yOffset", i64::from(at.y_offset));
			t.set("x", i64::from(x));
			t.set("y", i64::from(y));
		}
		t.to_strong()
	}

	/// `render()` (`0x4a4030`): draw every visible overlay at real time `now`, committing
	/// finished fades (one that faded out is hidden after). What it drew is what
	/// [`EntityOverlays::drawn`] gives until the next frame begins.
	pub(crate) fn render(&mut self, now: f32) {
		self.drawn.clear();
		for o in self.list.iter_mut().filter(|o| o.visible) {
			let drawn = o.color.draw(now);
			if drawn.hide {
				o.visible = false;
			}
			self.drawn.push(SceneOrientationOverlay {
				brush: o.brush.clone(),
				pos: o.pos,
				scale: o.scale,
				color: drawn.color,
			});
		}
	}

	/// A new frame: nothing drawn yet.
	pub(crate) fn begin_frame(&mut self) {
		self.drawn.clear();
	}

	/// What this frame's `render()` drew.
	pub(crate) fn drawn(&self) -> &[SceneOrientationOverlay] {
		&self.drawn
	}

	/// `queryEntityOverlays()` on a camera that places nothing (the world's): per
	/// overlay, not shown, not offscreen, at `(0, 0)`.
	pub(crate) fn states(&self, vm: &SQVM) -> SqStrongRef {
		let mut list = SqArray::new(vm);
		for i in 0..self.list.len() {
			list.push(self.state(vm, i, Located::default(), (0, 0)));
		}
		list.to_strong()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn overlays(size: (i32, i32)) -> EntityOverlays {
		EntityOverlays {
			list: vec![Overlay {
				id: 7,
				identity: 1,
				brush: "zombie_01_orientation".into(),
				size,
				scale: 1.0,
				color: SpriteColor::default(),
				visible: false,
				shown: false,
				pos: (0.0, 0.0),
			}],
			drawn: Vec::new(),
		}
	}

	fn shown(x: i32, y: i32, y_offset: i32) -> Located {
		Located {
			x,
			y,
			visible: true,
			offscreen: false,
			y_offset,
		}
	}

	#[test]
	fn a_shown_overlay_is_drawn_where_its_tile_is_lowered_by_its_offset() {
		let mut o = overlays((46, 46));
		assert_eq!(o.place(0, shown(980, 710, 40), (2560, 1440), 0.0), (980, 710));
		// 980 - 1280 and 1440 - 750 - 720, y up.
		assert_eq!(o.list[0].pos, (-300.0, -30.0));
		assert!(o.list[0].visible);
	}

	#[test]
	fn an_overlay_reaches_at_most_ten_pixels_past_an_edge() {
		let mut o = overlays((46, 46));
		assert_eq!(o.place(0, shown(-500, 3000, 0), (2560, 1440), 0.0), (13, 1427));
		assert_eq!(o.place(0, shown(5000, -40, 0), (2560, 1440), 0.0), (2547, 13));
	}

	#[test]
	fn the_first_showing_is_opaque_at_once_and_a_hidden_one_fades_back_in() {
		let mut o = overlays((46, 46));
		o.place(0, shown(100, 100, 0), (2560, 1440), 0.0);
		o.render(0.0);
		assert_eq!(o.drawn()[0].color, 0xffff_ffff, "white and opaque from the start");
		// Hidden: fades out over 200 ms, then isn't drawn.
		o.place(0, Located::default(), (2560, 1440), 1.0);
		o.render(1.1);
		assert_eq!(o.drawn()[0].color >> 24, 127);
		o.render(1.2);
		assert_eq!(o.drawn()[0].color >> 24, 0);
		o.render(1.3);
		assert!(o.drawn().is_empty());
		// Shown again: from transparent.
		o.place(0, shown(100, 100, 0), (2560, 1440), 2.0);
		o.render(2.05);
		assert_eq!(o.drawn()[0].color >> 24, 63);
	}

	#[test]
	fn an_overlay_that_does_not_show_reports_the_origin() {
		let mut o = overlays((46, 46));
		assert_eq!(o.place(0, Located::default(), (2560, 1440), 0.0), (0, 0));
		assert!(!o.list[0].visible);
	}
}
