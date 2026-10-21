//! Portraits: an entity drawn on its own into an image, for the UI's procedural images
//! `coui://procedural/tacticalentity(...)` and `worldentity(...)`. Each handler finds
//! only its own kind of entity ([`PortraitKind`]). The request names the entity by id;
//! the arguments after it are sprites to leave out (`socket`, `miniboss`, `arrow`).
//!
//! A battle unit's (`tacticalentity`), as the original's handler `0x4d3e80` draws it
//! (rendering through `0x4ac7d0`):
//! - The image is twice the farthest horizontal extent of any drawn sprite's brush
//!   wide, scaled, and twice the farthest vertical one high (`0x4a8fd0`), so the
//!   entity's anchor is its centre.
//! - Sprites draw back to front at the anchor, without their brushes' offsets, at
//!   their own offsets only with `setAlwaysApplySpriteOffset`, fully opaque, onto
//!   transparency (`0x4a8700`). A sprite with `setSpriteRenderToTexture(false)` is
//!   left out.
//!
//! A world entity's (`worldentity(rand,id)`, which `world_entity.getImagePath()`
//! returns; the original has no such handler, so these rules are ours,
//! `ModernEngine.Version` 3): the entity as the world map shows it at zoom 1, cut out.
//! - Drawn: its sprites that are visible, have a brush, are drawn into portraits
//!   (`setSpriteRenderToTexture`) and aren't transparent (`Alpha` 0: a camp's fire by
//!   day, the player's zoomed-out banner). Left out: those that keep their screen size
//!   at any zoom (`setSpriteScaling(name, false)`: the selection ring, a location's
//!   banner), which mark the entity on the map rather than show it.
//! - Each sits where the map puts it: its slot's offset plus its brush's, both scaled
//!   with it, mirrored and turned about its anchor; its brush's first frame.
//! - In the map's order: a higher brush depth bias first, then the order they were
//!   added (the entities layer's sort, within one entity).
//! - In its own colour and alpha (what `Color` and `Alpha` read), without the day's
//!   light.
//! - The image is the drawn sprites' bounding box, rounded out to whole pixels.

use bb_assets::SpriteRegistry;
pub use squirrel_bb::scene::PortraitKind;
use squirrel_bb::scene::{PortraitSprite, ScenePortrait};

use crate::sprite::Quad;
use crate::world::{AtlasList, Look, WorldQuads, frame_quad, sprite_quad};

/// The handler's name a procedural image path gives: the text before its `(`,
/// percent-encoding allowed.
pub fn handler_name(path: &str) -> String {
	let path = percent_decode(path);
	path.split('(').next().unwrap_or_default().to_string()
}

/// The handler a procedural image path names (`tacticalentity` or `worldentity`, see
/// [`handler_name`]); `None` for any other.
pub fn handler(path: &str) -> Option<PortraitKind> {
	match handler_name(path).as_str() {
		"tacticalentity" => Some(PortraitKind::Tactical),
		"worldentity" => Some(PortraitKind::World),
		_ => None,
	}
}

/// A parsed procedural image path, e.g. `tacticalentity(3,10042,socket,miniboss,arrow)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortraitRequest {
	pub kind: PortraitKind,
	pub id: i64,
	/// The sprites to leave out.
	pub hidden: Vec<String>,
}

impl PortraitRequest {
	/// Parse `path` (after `coui://procedural/`, percent-encoding allowed). The first
	/// argument (a content id, or a random number to defeat caching) is ignored.
	pub fn parse(path: &str) -> Option<PortraitRequest> {
		let path = percent_decode(path);
		let kind = handler(&path)?;
		let open = path.find('(')?;
		let args = path[open + 1..].strip_suffix(')')?;
		let mut args = args.split(',').map(str::trim);
		let _content = args.next()?;
		let id = args.next()?.parse().ok()?;
		Some(PortraitRequest {
			kind,
			id,
			hidden: args.filter(|a| !a.is_empty()).map(str::to_string).collect(),
		})
	}
}

/// `%XX` escapes decoded (the page may encode the parentheses and commas).
fn percent_decode(s: &str) -> String {
	let bytes = s.as_bytes();
	let mut out = Vec::with_capacity(bytes.len());
	let mut i = 0;
	while i < bytes.len() {
		let hex = |b: u8| (b as char).to_digit(16);
		if bytes[i] == b'%' && i + 2 < bytes.len() {
			if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
				out.push((h * 16 + l) as u8);
				i += 3;
				continue;
			}
		}
		out.push(bytes[i]);
		i += 1;
	}
	String::from_utf8_lossy(&out).into_owned()
}

/// The sprites a portrait draws, in order.
fn drawn<'a>(portrait: &'a ScenePortrait, hidden: &[String], sprites: &SpriteRegistry) -> Vec<&'a PortraitSprite> {
	let shown = |s: &&PortraitSprite| s.visible && s.in_portrait && !s.brush.is_empty() && !hidden.contains(&s.name);
	match portrait.kind {
		PortraitKind::Tactical => portrait.sprites.iter().filter(shown).collect(),
		PortraitKind::World => {
			let mut drawn: Vec<&PortraitSprite> = portrait
				.sprites
				.iter()
				.filter(shown)
				.filter(|s| !s.keeps_screen_size && s.color >> 24 != 0)
				.collect();
			// The entities layer draws a higher key first, ties in the order added; within
			// one entity the keys differ by the brushes' depth biases (a stable sort).
			drawn.sort_by_key(|s| std::cmp::Reverse(sprites.get(&s.brush).map_or(0, |(_, b)| b.depth_bias)));
			drawn
		}
	}
}

/// A portrait image's size in pixels, and the point at its centre (world units from
/// the entity's anchor, y up).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PortraitFrame {
	pub width: u32,
	pub height: u32,
	pub center: [f32; 2],
}

/// The image `portrait` is drawn into (see the module's rules); `width` and `height`
/// 0 for nothing to draw.
pub fn portrait_frame(portrait: &ScenePortrait, hidden: &[String], sprites: &SpriteRegistry) -> PortraitFrame {
	match portrait.kind {
		PortraitKind::Tactical => {
			let (width, height) = unit_size(portrait, hidden, sprites);
			PortraitFrame { width, height, center: [0.0, 0.0] }
		}
		PortraitKind::World => {
			let quads = portrait_quads(portrait, hidden, sprites);
			let mut bounds: Option<[f32; 4]> = None;
			for [x, y] in quads.quads.iter().flat_map(|q| q.corners) {
				let b = bounds.get_or_insert([x, x, y, y]);
				*b = [b[0].min(x), b[1].max(x), b[2].min(y), b[3].max(y)];
			}
			let Some([left, right, bottom, top]) = bounds else {
				return PortraitFrame { width: 0, height: 0, center: [0.0, 0.0] };
			};
			let (left, right, bottom, top) = (left.floor(), right.ceil(), bottom.floor(), top.ceil());
			PortraitFrame {
				width: (right - left) as u32,
				height: (top - bottom) as u32,
				center: [(left + right) / 2.0, (bottom + top) / 2.0],
			}
		}
	}
}

/// The image's size in pixels (see [`portrait_frame`]).
pub fn portrait_size(portrait: &ScenePortrait, hidden: &[String], sprites: &SpriteRegistry) -> (u32, u32) {
	let frame = portrait_frame(portrait, hidden, sprites);
	(frame.width, frame.height)
}

/// A battle unit's image size: twice the farthest extent of a drawn sprite's brush on
/// each axis, times its scale, truncated (`0x4a8fd0`). `(0, 0)` for nothing to draw.
fn unit_size(portrait: &ScenePortrait, hidden: &[String], sprites: &SpriteRegistry) -> (u32, u32) {
	let mut size = (0u32, 0u32);
	for sprite in drawn(portrait, hidden, sprites) {
		let Some((_, brush)) = sprites.get(&sprite.brush) else {
			continue;
		};
		let [left, right, top, bottom] = brush.bounds();
		let w = (2.0 * left.abs().max(right.abs()) * sprite.scale) as u32;
		let h = (2.0 * top.abs().max(bottom.abs()) * sprite.scale) as u32;
		size = (size.0.max(w), size.1.max(h));
	}
	size
}

/// The quads that draw `portrait` around the entity's anchor (the origin).
pub fn portrait_quads(portrait: &ScenePortrait, hidden: &[String], sprites: &SpriteRegistry) -> WorldQuads {
	let mut atlases = AtlasList::default();
	let mut quads = Vec::new();
	for sprite in drawn(portrait, hidden, sprites) {
		let [alpha, r, g, b] = sprite.color.to_be_bytes();
		let look = Look {
			scale: sprite.scale,
			flipped: sprite.flipped,
			alpha: 255.0,
			rotation: sprite.rotation,
			tint: [r, g, b],
		};
		let placed = match portrait.kind {
			PortraitKind::Tactical => sprites.get(&sprite.brush).and_then(|(sheet, brush)| {
				// The frames sit on the anchor; the brush's own offset is where a unit's
				// sprites stand on the map, not part of the picture (a bust's head reaches
				// past the image with it).
				let offset = if portrait.apply_offsets { sprite.offset } else { (0.0, 0.0) };
				let frame = brush.frames.first()?;
				Some((sheet.sheet.texture.as_str(), frame_quad(frame, [offset.0, offset.1], look)))
			}),
			PortraitKind::World => {
				let look = Look { alpha: f32::from(alpha), ..look };
				let offset = [sprite.offset.0, sprite.offset.1];
				sprite_quad(sprites, &sprite.brush, 0, [0.0, 0.0], offset, look).map(|(texture, _, quad)| (texture, quad))
			}
		};
		let Some((texture, quad)) = placed else {
			continue;
		};
		quads.push(Quad {
			texture: atlases.index_of(texture),
			saturation: sprite.saturation,
			flags: 1,
			..quad
		});
	}
	let labels_from = quads.len();
	WorldQuads {
		quads,
		atlases,
		labels_from,
		details_end: 0,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn parses_the_scripts_paths() {
		assert_eq!(
			PortraitRequest::parse("tacticalentity(3,10042,socket,miniboss,arrow)"),
			Some(PortraitRequest {
				kind: PortraitKind::Tactical,
				id: 10042,
				hidden: vec!["socket".into(), "miniboss".into(), "arrow".into()],
			})
		);
		assert_eq!(
			PortraitRequest::parse("worldentity(123456,10007)"),
			Some(PortraitRequest {
				kind: PortraitKind::World,
				id: 10007,
				hidden: Vec::new(),
			})
		);
		assert_eq!(
			PortraitRequest::parse("tacticalentity%283%2C10042%2Csocket%29").map(|r| r.hidden),
			Some(vec!["socket".to_string()])
		);
		assert_eq!(PortraitRequest::parse("tacticalentity(3)"), None);
		assert_eq!(PortraitRequest::parse("other(1,2)"), None);
	}

	#[test]
	fn names_the_handler_even_with_arguments_it_cannot_read() {
		assert_eq!(handler("tacticalentity(3)"), Some(PortraitKind::Tactical));
		assert_eq!(handler("tacticalentity(0,abc)"), Some(PortraitKind::Tactical));
		assert_eq!(handler("worldentity%28123%2C7%29"), Some(PortraitKind::World));
		assert_eq!(handler("bogus(1,2)"), None);
		assert_eq!(handler_name("bogus%281%2C2%29"), "bogus");
		assert_eq!(handler("tacticalentityx(1,2)"), None);
	}
}
