//! Which mouse cursor shows, and who draws it.
//!
//! The original draws the scripts' cursor sprite into its frame (the software cursor,
//! see `squirrel_bb::native::ui_cursor`), or with its hardware cursor setting shows
//! `cursors/<name>.cur` as the system's cursor. We also show the sprite itself as the
//! system's cursor (a divergence, on by default for players; see `docs/differences.md`):
//! the compositor then moves it, and a mouse move over a still picture needs no frame
//! drawn. Each animation frame is a picture of its own, switched as the software cursor
//! would switch it (on virtual time).
//!
//! The system's cursor is always at the mouse, the software cursor where the scripts
//! last put it (`UI.setCursorPosition`, which the states call with the mouse's position
//! on every move). Nothing in vanilla or the mods surveyed puts it elsewhere, but if
//! the scripts do (or haven't followed the mouse, as behind the loading screen), the
//! game draws its cursor there until it is at the mouse again.
//!
//! The software cursor is drawn a pixel of its picture to a pixel of the game's screen
//! (`screen_scale`): unscaled in the window's pixels where the screen is in pixels, and
//! each pixel doubled on a Retina display, where it is in points. The system's shows
//! the same at any display scale (see `system_cursor`).

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use bb_assets::{CursorImage, FileSource, SpriteRegistry, decode_png};
use squirrel_bb::CursorLook;

/// What shows as the mouse cursor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CursorShow {
	/// The game draws the sprite into its frame (the original's software cursor).
	Drawn,
	/// The system shows frame `frame` of the sprite `brush` (see [`CursorPictures`]).
	Sprite { brush: String, frame: usize },
	/// The system shows the original's hardware cursor `cursors/<name>.cur` (the
	/// scripts' `UseHardwareCursor`).
	File(String),
	/// No cursor: the scripts set none, or one without a sprite.
	Hidden,
}

/// What shows for the scripts' cursor `look`: with `system` (the player's setting), as
/// the system's when the scripts' position is the mouse's (`at_mouse`) and the
/// sprite's frame `frame` has a picture (`None`: it has none).
pub fn choose(look: &CursorLook, system: bool, at_mouse: bool, frame: Option<usize>) -> CursorShow {
	if look.hardware {
		return CursorShow::File(look.name.clone());
	}
	let Some(brush) = &look.brush else { return CursorShow::Hidden };
	match frame {
		Some(frame) if system && at_mouse => CursorShow::Sprite { brush: brush.clone(), frame },
		_ => CursorShow::Drawn,
	}
}

/// The cursor sprites' pictures for the system, built on first use: every frame of a
/// sprite, cut from its atlas (see `bb_render::cursor_image`). The first build makes
/// those of every cursor the scripts name at once, so each atlas is decoded once.
pub struct CursorPictures {
	files: Arc<dyn FileSource>,
	/// By brush; `None`: it has none (its atlas missing, or too large a picture).
	built: HashMap<String, Option<Rc<[CursorImage]>>>,
}

impl CursorPictures {
	pub fn new(files: Arc<dyn FileSource>) -> CursorPictures {
		CursorPictures {
			files,
			built: HashMap::new(),
		}
	}

	/// The pictures of `brush`'s frames, built with those of `others` (the brushes the
	/// scripts name as cursors) the first time.
	pub fn get(&mut self, registry: &SpriteRegistry, brush: &str, others: impl FnOnce() -> Vec<String>) -> Option<Rc<[CursorImage]>> {
		if let Some(built) = self.built.get(brush) {
			return built.clone();
		}
		let mut names = if self.built.is_empty() { others() } else { Vec::new() };
		names.push(brush.to_string());
		self.build(registry, names);
		self.built.get(brush).cloned().flatten()
	}

	/// Build `names`' pictures (those not built yet), each atlas decoded once.
	fn build(&mut self, registry: &SpriteRegistry, names: Vec<String>) {
		let mut by_atlas: HashMap<&str, Vec<String>> = HashMap::new();
		for name in names {
			if self.built.contains_key(&name) {
				continue;
			}
			match registry.get(&name) {
				Some((sheet, _)) => by_atlas.entry(sheet.sheet.texture.as_str()).or_default().push(name),
				None => {
					self.built.insert(name, None);
				}
			}
		}
		for (path, names) in by_atlas {
			let atlas = self.files.read(path).and_then(|bytes| decode_png(&bytes).ok());
			for name in names {
				let pictures = atlas.as_ref().and_then(|atlas| {
					let (_, sprite) = registry.get(&name)?;
					bb_render::cursor_image::cursor_frames(sprite, atlas)
				});
				self.built.insert(name, pictures.map(Rc::from));
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use std::sync::atomic::{AtomicUsize, Ordering};

	use super::*;

	fn look(brush: Option<&str>, hardware: bool) -> CursorLook {
		CursorLook {
			brush: brush.map(str::to_string),
			hardware,
			name: "cursor_hand".into(),
			since: 0.0,
		}
	}

	#[test]
	fn the_system_shows_the_sprite_only_where_the_software_cursor_would_be() {
		let hand = look(Some("cursor_hand"), false);
		let sprite = CursorShow::Sprite {
			brush: "cursor_hand".into(),
			frame: 3,
		};
		assert_eq!(choose(&hand, true, true, Some(3)), sprite);
		assert_eq!(choose(&hand, false, true, Some(3)), CursorShow::Drawn, "the software cursor chosen");
		assert_eq!(choose(&hand, true, false, Some(3)), CursorShow::Drawn, "the scripts put it elsewhere");
		assert_eq!(choose(&hand, true, true, None), CursorShow::Drawn, "no picture for it");
		assert_eq!(choose(&look(None, false), true, true, None), CursorShow::Hidden);
		assert_eq!(choose(&look(None, false), false, true, None), CursorShow::Hidden);
		// The original's hardware cursor setting wins, as in the original.
		let file = CursorShow::File("cursor_hand".into());
		assert_eq!(choose(&look(Some("cursor_hand"), true), true, true, Some(0)), file);
		assert_eq!(choose(&look(Some("cursor_hand"), true), false, false, None), file);
	}

	/// The files, counting reads (each atlas is decoded once).
	struct Files(HashMap<String, Vec<u8>>, AtomicUsize);

	impl FileSource for Files {
		fn read(&self, path: &str) -> Option<Vec<u8>> {
			self.1.fetch_add(1, Ordering::Relaxed);
			self.0.get(path).cloned()
		}
	}

	#[test]
	fn pictures_are_built_once_per_atlas_and_kept() {
		use bb_assets::{BrushFrame, BrushSheet, BrushSprite, SheetHeader, name_hash};
		let sprite = |name: &str, frames: usize| BrushSprite {
			hash: name_hash(name.as_bytes()),
			name: name.into(),
			material: 0,
			width: 2,
			height: 2,
			offset_x: 0,
			offset_y: 0,
			depth_bias: 0,
			frame_time_ms: 50,
			unknown_pause: [0, 0],
			rotation_speed: 0.0,
			flags: 0,
			pivot: [0.0, 0.0],
			color: [255; 4],
			unknown_scale: 1.0,
			frames: (0..frames)
				.map(|i| BrushFrame {
					name: format!("{i}"),
					uv: [0.0, 1.0, 0.0, 1.0],
					rect: [-1.0, 1.0, -1.0, 1.0],
				})
				.collect(),
			trailer: [0; 3],
		};
		let mut registry = SpriteRegistry::new();
		let header = SheetHeader {
			width: 2,
			height: 2,
			flags: [0; 12],
			sprite_count: 3,
			reserved: [0; 2],
			max_level: 1000,
		};
		registry.add_sheet(
			"brushes/ui.brush",
			BrushSheet {
				header,
				texture: "gfx/ui.png".into(),
				sprites: vec![sprite("cursor_hand", 1), sprite("cursor_attack", 10), sprite("cursor_lost", 1)],
			},
		);
		let mut png = Vec::new();
		let atlas = bb_assets::Image {
			width: 2,
			height: 2,
			rgba: vec![200; 16],
		};
		bb_render::write_png(&mut png, &atlas).unwrap();
		let files = Arc::new(Files(HashMap::from([("gfx/ui.png".to_string(), png)]), AtomicUsize::new(0)));
		let mut pictures = CursorPictures::new(files.clone());
		let others = || vec!["cursor_attack".to_string(), "cursor_unknown".to_string()];
		let hand = pictures.get(&registry, "cursor_hand", others).expect("pictures");
		assert_eq!((hand.len(), hand[0].width, hand[0].hotspot), (1, 2, (1, 1)));
		let attack = pictures.get(&registry, "cursor_attack", || unreachable!("built already")).expect("built with it");
		assert_eq!(attack.len(), 10);
		assert_eq!(files.1.load(Ordering::Relaxed), 1, "the atlas decoded once");
		assert!(pictures.get(&registry, "cursor_unknown", Vec::new).is_none());
		assert!(pictures.get(&registry, "cursor_lost", Vec::new).is_some(), "built on its own later");
		assert_eq!(files.1.load(Ordering::Relaxed), 2);
	}
}
