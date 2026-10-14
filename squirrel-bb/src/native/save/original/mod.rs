//! The original's campaign save layout, disassembled from its `World.save`
//! (`0x4f4650`) and `World.load` (`0x4f4ac0`). After the container's header (see
//! [`super::container`]):
//!
//! 1. The virtual time: `f64` (an `f32` up to version 49).
//! 2. The world scene ([`scene`]): the tiles, the terrain's brushes, every world entity's
//!    native part (position, flags, faction, sprites, its script's `ClassNameHash`),
//!    the decorations, footprints and region names.
//! 3. The world camera and clock ([`camera`]).
//! 4. The rosters: the player's, the guests', then every `createRoster` roster by id
//!    (`u32` id, `u16` size). Each unit is a record ([`record`]) whose native part is
//!    followed by its script's `onSerialize`.
//! 5. Every world entity's script data, by id descending: `u32` id, its `onSerialize`.
//! 6. `onSave`: the world state's `onSerialize` (the managers).
//! 7. The fog of war ([`fog`]).
//!
//! The load clears the world, runs `onBeforeLoad`, then reads in that order: the scene
//! creates every entity from its script (`create`, `onInit`, `onAfterInit`) and puts its
//! saved sprites back by name; units are created and deserialized one by one; the world
//! entities' scripts are deserialized after every roster; then `onLoad`.
//!
//! The modules here read and write the native records as data; [`import`] runs a load
//! through the engine's [`OriginalWorld`](world::OriginalWorld).

pub(crate) mod camera;
pub(crate) mod fog;
pub(crate) mod game_entity;
pub(crate) mod import;
pub(crate) mod record;
pub(crate) mod scene;
pub(crate) mod sprite;
pub(crate) mod wire;
pub(crate) mod world;

#[cfg(test)]
mod tests {
	use super::super::container;
	use super::super::stream::{SaveWriter, StrLen};
	use super::camera::{Camera, Clock};
	use super::scene::Scene;

	/// The original's saves in its Steam (Proton) install (read only; skipped where there
	/// are none).
	const SAVES: &str = concat!(
		env!("HOME"),
		"/.local/share/Steam/steamapps/compatdata/365360/pfx/drive_c/users/steamuser/Documents/Battle Brothers/savegames"
	);

	/// Every campaign save the original wrote (from version 63 on) reads, and writes
	/// back the same bytes, through the scene, the camera and the clock.
	#[test]
	fn the_originals_saves_read_and_write_back_exactly() {
		let Ok(dir) = std::fs::read_dir(SAVES) else {
			eprintln!("skipping: no saves at {SAVES}");
			return;
		};
		let mut checked = 0;
		for entry in dir.flatten() {
			let name = entry.file_name().to_string_lossy().into_owned();
			let Some(stem) = name.strip_suffix(".sav") else { continue };
			let bytes = std::fs::read(entry.path()).unwrap();
			let Ok((meta, mut r)) = container::decode(bytes.clone(), stem) else { continue };
			// Storages (mods' data) aren't campaigns, and engine builds before 2020-09-13 wrote
			// the virtual time and the tiles differently (the current original can't read
			// those either).
			if meta.version < 63 || !meta.has_data("days") || meta.created < 1_599_955_200 {
				continue;
			}
			let start = r.pos();
			r.f64().unwrap();
			let scene = Scene::read(&mut r).unwrap_or_else(|e| panic!("{name}: {e}"));
			let camera = Camera::read(&mut r).unwrap();
			let clock = Clock::read(&mut r).unwrap();
			let end = r.pos();
			let mut w = SaveWriter {
				strings: StrLen::U16,
				..SaveWriter::default()
			};
			w.buf.extend_from_slice(&bytes[start..start + 8]);
			scene.write(&mut w);
			camera.write(&mut w);
			clock.write(&mut w);
			assert!(w.buf == bytes[start..end], "{name}: written back differently");
			assert!(clock.phase < 10 && camera.zoom > 0.0, "{name}: camera {camera:?} clock {clock:?}");
			checked += 1;
		}
		eprintln!("{checked} saves");
	}
}
