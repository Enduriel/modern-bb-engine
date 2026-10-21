//! The highlighter's overlay icons (`addOverlayIcon(brush, tile, x, y)`, `0x4b5200`;
//! `clearOverlayIcons()`, `0x4b5350`): an area attack's tiles, a ranged shot's blocked
//! tiles. An icon stands at world `(x, y)` (the `tile` is ignored) in the battle's
//! effects layer, over the units, white and opaque. Cleared, it fades out over 100 ms of
//! real time and then goes.

use crate::native::sprite_color::SpriteColor;
use crate::scene::tactical::SceneOverlayIcon;

/// How long a cleared icon takes to fade out, in ms.
const FADE_OUT: i64 = 100;

struct Icon {
	brush: String,
	pos: (f32, f32),
	color: SpriteColor,
	/// Fading out after a clear.
	cleared: bool,
}

#[derive(Default)]
pub(super) struct Icons(Vec<Icon>);

impl Icons {
	pub fn add(&mut self, brush: String, pos: (f32, f32)) {
		self.0.push(Icon {
			brush,
			pos,
			color: SpriteColor::new(0xffff_ffff),
			cleared: false,
		});
	}

	/// `clearOverlayIcons()` at real time `now`.
	pub fn clear(&mut self, now: f32) {
		for icon in self.0.iter_mut().filter(|i| !i.cleared) {
			icon.color.fade_out(FADE_OUT, now, false);
			icon.cleared = true;
		}
	}

	/// Whether one fades at real time `now`.
	pub fn fading(&self, now: f32) -> bool {
		self.0.iter().any(|i| i.color.is_fading(now))
	}

	/// The icons as drawn at real time `now`; the faded-out ones go.
	pub fn scene(&mut self, now: f32) -> Vec<SceneOverlayIcon> {
		self.0.retain_mut(|i| {
			let drawn = i.color.draw(now);
			!(i.cleared && drawn.color >> 24 == 0 && i.color.is_fading_done())
		});
		self.0
			.iter()
			.map(|i| SceneOverlayIcon {
				brush: i.brush.clone(),
				pos: i.pos,
				color: i.color.shown(now),
				tile: None,
			})
			.collect()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn cleared_icons_fade_out_then_go() {
		let mut icons = Icons::default();
		icons.add("overlay_aoe_attack".into(), (10.0, 20.0));
		assert_eq!(icons.scene(0.0)[0].color, 0xffff_ffff);
		icons.clear(1.0);
		let half = icons.scene(1.05)[0].color >> 24;
		assert!(half > 0x70 && half < 0x90, "half faded: {half:#x}");
		assert!(icons.scene(1.2).is_empty());
	}
}
