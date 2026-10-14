//! Portraits: an entity drawn on its own into an image, for the UI's procedural images
//! `coui://procedural/tacticalentity(...)` and `worldentity(...)` (the original's
//! handler `0x4d3e80`, rendering through `0x4ac7d0`; disassembled 2026-10-14).
//!
//! - The request names the entity by id; the arguments after it are sprites to leave
//!   out (`socket`, `miniboss`, `arrow`).
//! - The image is twice the farthest horizontal extent of any drawn sprite's brush
//!   wide, scaled, and twice the farthest vertical one high (`0x4a8fd0`), so the
//!   entity's anchor is its centre.
//! - Sprites draw back to front at the anchor, without their brushes' offsets, at
//!   their own offsets only with `setAlwaysApplySpriteOffset`, fully opaque, onto
//!   transparency (`0x4a8700`). A sprite with `setSpriteRenderToTexture(false)` is
//!   left out.

use bb_assets::SpriteRegistry;
use squirrel_bb::scene::ScenePortrait;

use crate::sprite::Quad;
use crate::world::{AtlasList, Look, WorldQuads, frame_quad};

/// Which kind of entity a procedural image shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortraitKind {
	/// `tacticalentity(...)`: a battle unit (also a brother in the roster).
	Tactical,
	/// `worldentity(...)`: a world entity.
	World,
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
		let open = path.find('(')?;
		let kind = match &path[..open] {
			"tacticalentity" => PortraitKind::Tactical,
			"worldentity" => PortraitKind::World,
			_ => return None,
		};
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
fn drawn<'a>(
	portrait: &'a ScenePortrait,
	hidden: &'a [String],
) -> impl Iterator<Item = &'a squirrel_bb::scene::PortraitSprite> + 'a {
	portrait
		.sprites
		.iter()
		.filter(|s| s.visible && s.in_portrait && !s.brush.is_empty() && !hidden.contains(&s.name))
}

/// The image's size: twice the farthest extent of a drawn sprite's brush on each axis,
/// times its scale, truncated (`0x4a8fd0`). `(0, 0)` for nothing to draw.
pub fn portrait_size(portrait: &ScenePortrait, hidden: &[String], sprites: &SpriteRegistry) -> (u32, u32) {
	let mut size = (0u32, 0u32);
	for sprite in drawn(portrait, hidden) {
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

/// The quads that draw `portrait` around the origin (the image's centre).
pub fn portrait_quads(portrait: &ScenePortrait, hidden: &[String], sprites: &SpriteRegistry) -> WorldQuads {
	let mut atlases = AtlasList::default();
	let mut quads = Vec::new();
	for sprite in drawn(portrait, hidden) {
		let Some((sheet, brush)) = sprites.get(&sprite.brush) else {
			continue;
		};
		let Some(frame) = brush.frames.first() else {
			continue;
		};
		let [_, r, g, b] = sprite.color.to_be_bytes();
		let look = Look {
			scale: sprite.scale,
			flipped: sprite.flipped,
			alpha: 255.0,
			rotation: sprite.rotation,
			tint: [r, g, b],
		};
		// The frames sit on the anchor; the brush's own offset is where a unit's sprites
		// stand on the map, not part of the picture (a bust's head reaches past the
		// image with it).
		let offset = if portrait.apply_offsets { sprite.offset } else { (0.0, 0.0) };
		let anchor = [offset.0, offset.1];
		quads.push(Quad {
			texture: atlases.index_of(&sheet.sheet.texture),
			saturation: sprite.saturation,
			flags: 1,
			..frame_quad(frame, anchor, look)
		});
	}
	let labels_from = quads.len();
	WorldQuads {
		quads,
		atlases,
		labels_from,
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
}
