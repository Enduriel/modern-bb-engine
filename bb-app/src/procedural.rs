//! The page's procedural images (`coui://procedural/<path>`): entity portraits, and
//! what a request gets when there is none to draw. Probed in the original 2026-10-21
//! (an image and an XHR each, campaign mode):
//!
//! - `tacticalentity(content,id,hidden…)`: battle unit `id`'s portrait (the original's
//!   only handler). It finds battle units only (a brother in the roster included). For
//!   any other id (none, a world entity's, a settlement's), arguments it can't read
//!   (`tacticalentity()`, an id `abc`) or a unit with nothing to draw, the original
//!   answers 200 with its fallback image ([`FALLBACK_PNG`]) and logs the warning
//!   `Failed to load procedural content "procedural/<path>".` (tag `UI`).
//! - `worldentity(rand,id,hidden…)`: world entity `id`'s portrait. Ours: the original
//!   has no such handler (`ModernEngine.Version` 3 says this engine has). It finds
//!   world entities only; for any other id, or one with nothing to draw, the load
//!   fails as the original's does for every `worldentity` image, logged as the
//!   `tacticalentity` failure is.
//! - Any other handler: the load fails (an image's `error`, an XHR's status 0), and
//!   the error `Failed to find procedural content handler "<name>".` is logged (tag
//!   `UI`), as in the original.

use bb_engine::harness::GameEngine;
use bb_render::portrait::{PortraitKind, PortraitRequest, handler, handler_name};
use bb_render::{Gpu, WorldRenderer, write_png};
use squirrel_bb::html_log::{self, Class};

/// What a procedural image request gets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Procedural {
	/// A PNG.
	Image(Vec<u8>),
	/// A failed load (an image's `error`, an XHR's status 0).
	Fail,
}

/// The original's image for a `tacticalentity(...)` it can't draw, byte for byte: a
/// 16×16 PNG (made with Adobe ImageReady) of white at alpha 0, so nothing shows.
/// Read back as bytes through an XHR in the original (2026-10-21).
pub const FALLBACK_PNG: &[u8] = &[
	0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
	0x00, 0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x10, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0xf3, 0xff,
	0x61, 0x00, 0x00, 0x00, 0x19, 0x74, 0x45, 0x58, 0x74, 0x53, 0x6f, 0x66, 0x74, 0x77, 0x61, 0x72,
	0x65, 0x00, 0x41, 0x64, 0x6f, 0x62, 0x65, 0x20, 0x49, 0x6d, 0x61, 0x67, 0x65, 0x52, 0x65, 0x61,
	0x64, 0x79, 0x71, 0xc9, 0x65, 0x3c, 0x00, 0x00, 0x00, 0x1f, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda,
	0x62, 0xfc, 0xff, 0xff, 0x3f, 0x03, 0x25, 0x80, 0x89, 0x81, 0x42, 0x30, 0x6a, 0xc0, 0xa8, 0x01,
	0xa3, 0x06, 0x0c, 0x16, 0x03, 0x00, 0x02, 0x0c, 0x00, 0x63, 0x9a, 0x03, 0x1d, 0x5b, 0x62, 0xe3,
	0xc6, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

/// The answer to `path` (after `procedural/`, still percent-encoded). Like the
/// original's, a portrait's pixels are as drawn (colour premultiplied), not
/// unpremultiplied for the PNG.
pub fn answer(renderer: &mut WorldRenderer, gpu: &Gpu, engine: &GameEngine, path: &str) -> Procedural {
	let Some(kind) = handler(path) else {
		let text = format!("Failed to find procedural content handler \"{}\".", handler_name(path));
		html_log::line(Class::Error, "UI", text);
		return Procedural::Fail;
	};
	if let Some(png) = portrait_png(renderer, gpu, engine, path) {
		return Procedural::Image(png);
	}
	html_log::line(Class::Warning, "UI", format!("Failed to load procedural content \"procedural/{path}\"."));
	match kind {
		PortraitKind::Tactical => Procedural::Image(FALLBACK_PNG.to_vec()),
		PortraitKind::World => Procedural::Fail,
	}
}

/// The portrait's PNG, or `None` for no entity of the path's kind with its id, or
/// nothing to draw.
fn portrait_png(renderer: &mut WorldRenderer, gpu: &Gpu, engine: &GameEngine, path: &str) -> Option<Vec<u8>> {
	let request = PortraitRequest::parse(path)?;
	let portrait = engine.portrait(request.kind, request.id)?;
	let image = engine
		.sprites()
		.with(|registry| renderer.compose_portrait(gpu, &portrait, &request.hidden, registry));
	let image = match image {
		Ok(image) => image?,
		Err(e) => {
			eprintln!("portrait {path}: {e}");
			return None;
		}
	};
	let mut png = Vec::new();
	write_png(&mut png, &image).ok()?;
	Some(png)
}
