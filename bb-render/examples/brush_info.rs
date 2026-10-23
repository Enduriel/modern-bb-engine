//! Print what an install's brushes say about named sprites (or every sprite whose name
//! starts with a prefix ending in `*`): size, offsets, depth bias, flags, frames.
//!
//! ```text
//! cargo run -p bb-render --example brush_info -- NAME... [PREFIX*]
//! ```

use std::path::Path;

use bb_engine::DatArchive;
use bb_engine::assets::load_sprites;
use bb_engine::harness::{self, STEAM_DATA_DIR};

fn main() {
	let paths = harness::data_archives(Path::new(STEAM_DATA_DIR)).expect("list archives");
	let mut archives: Vec<DatArchive> =
		paths.iter().map(|p| DatArchive::open(p).unwrap()).collect();
	let registry = load_sprites(&mut archives).registry;
	for arg in std::env::args().skip(1) {
		let matches: Vec<(String, String)> = match arg.strip_suffix('*') {
			Some(prefix) => registry
				.sheets()
				.iter()
				.flat_map(|s| {
					s.sheet
						.sprites
						.iter()
						.map(move |sp| (sp.name.clone(), s.path.clone()))
				})
				.filter(|(n, _)| n.starts_with(prefix))
				.collect(),
			None => vec![(arg.clone(), String::new())],
		};
		for (name, _) in matches {
			let Some((sheet, s)) = registry.get(&name) else {
				println!("{name}: unknown");
				continue;
			};
			println!(
				"{name}: {}x{} offset ({}, {}) pivot {:?} depth_bias {} flags {:#x} rotation_speed {} frames {} texture {}",
				s.width,
				s.height,
				s.offset_x,
				s.offset_y,
				s.pivot,
				s.depth_bias,
				s.flags,
				s.rotation_speed,
				s.frames.len(),
				sheet.sheet.texture
			);
			if let Some(f) = s.frames.first() {
				println!("    rect {:?} uv {:?}", f.rect, f.uv);
			}
		}
	}
}
