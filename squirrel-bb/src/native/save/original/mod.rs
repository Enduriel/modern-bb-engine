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

/// The largest world map the original's engine makes: `World.resizeScene` (`0x4efac0`)
/// clamps the scene to 140 columns and 170 rows (with the desert DLC). Its format can't
/// describe a larger map (the variable world size), whose saves stay in our own.
pub(crate) const MAX_MAP: (i64, i64) = (140, 170);

pub(crate) mod camera;
pub mod compare;
pub(crate) mod export;
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

	/// Read save `bytes` (named `name`) through the scene, the camera and the clock and
	/// write those back: `Ok(true)` if they come out the same, `Ok(false)` if it isn't a
	/// campaign save of the current format, `Err` if it fails to read or writes back
	/// differently.
	fn round_trip(name: &str, bytes: Vec<u8>) -> Result<bool, String> {
		let stem = name.strip_suffix(".sav").unwrap_or(name);
		let Ok((meta, mut r)) = container::decode(bytes.clone(), stem) else { return Ok(false) };
		// Storages (mods' data) aren't campaigns, and engine builds before 2020-09-13 wrote
		// the virtual time and the tiles differently (the current original can't read
		// those either).
		if meta.version < 63 || !meta.has_data("days") || meta.created < 1_599_955_200 {
			return Ok(false);
		}
		let start = r.pos();
		r.f64()?;
		let scene = Scene::read(&mut r)?;
		let camera = Camera::read(&mut r)?;
		let clock = Clock::read(&mut r)?;
		let end = r.pos();
		let mut w = SaveWriter {
			strings: StrLen::U16,
			..SaveWriter::default()
		};
		w.buf.extend_from_slice(&bytes[start..start + 8]);
		scene.write(&mut w);
		camera.write(&mut w);
		clock.write(&mut w);
		if w.buf != bytes[start..end] {
			return Err("written back differently".into());
		}
		if !(clock.phase < 10 && camera.zoom > 0.0) {
			return Err(format!("camera {camera:?} clock {clock:?}"));
		}
		Ok(true)
	}

	/// The `.sav` files in `dir`, by name.
	fn saves_in(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
		let mut all: Vec<_> = std::fs::read_dir(dir)
			.into_iter()
			.flatten()
			.flatten()
			.map(|e| e.path())
			.filter(|p| p.extension().is_some_and(|e| e == "sav"))
			.collect();
		all.sort();
		all
	}

	/// Every campaign save the original wrote (from version 63 on) reads, and writes
	/// back the same bytes, through the scene, the camera and the clock: the kept
	/// fixtures, and whatever is in the original's savegames folder at the time.
	#[test]
	fn the_originals_saves_read_and_write_back_exactly() {
		let fixtures = crate::platform::original_save_fixtures().filter(|d| d.is_dir());
		let mut checked = 0;
		for path in fixtures.as_deref().map(saves_in).unwrap_or_default() {
			let name = path.file_name().unwrap().to_string_lossy().into_owned();
			let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
			if round_trip(&name, bytes).unwrap_or_else(|e| panic!("{name}: {e}")) {
				checked += 1;
			}
		}
		eprintln!("{checked} kept saves");
		// More coverage: the saves in the original's savegames folder. The game and the
		// probes write and delete files there meanwhile, so a file that changes or goes
		// while it's read is passed over; one that stays the same must read.
		let Some(live) = crate::platform::original_savegames_dir().filter(|d| d.is_dir()) else {
			eprintln!("no savegames folder of the original's");
			return;
		};
		let mut live_checked = 0;
		for path in saves_in(&live) {
			let name = path.file_name().unwrap().to_string_lossy().into_owned();
			let Ok(bytes) = std::fs::read(&path) else { continue };
			match round_trip(&name, bytes.clone()) {
				Ok(true) => live_checked += 1,
				Ok(false) => {}
				Err(e) if std::fs::read(&path).is_ok_and(|again| again == bytes) => panic!("{name}: {e}"),
				Err(_) => eprintln!("{name}: changed while read, passed over"),
			}
		}
		eprintln!("{live_checked} saves in {}", live.display());
	}
}
