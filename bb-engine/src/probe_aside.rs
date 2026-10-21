//! The installed mods and mod storages as they are outside probe runs. A probe run (the
//! runner that launches the original game; not part of this repository) moves the
//! installed files aside for its duration and puts them back afterwards, so what is in
//! `data/` and `savegames/` during a run is not what was installed:
//!
//! - every mod but Modern Hooks is moved from `data/` into [`HELD_MODS`] beside it;
//! - `--with-mod` copies other zips into `data/` (a mod under development, Legends, a
//!   held mod), each recorded by name in [`WITH_MODS_STATE`] before it lands and removed
//!   with its record afterwards;
//! - runs with mods snapshot `savegames/` into [`SAVES_SNAPSHOT`] (stamped once
//!   complete) and put it back afterwards, so the mods' writes there are the run's;
//! - `--with-mod` runs move the installed mod storages (`MSU#….sav`) into [`HELD_SETTINGS`],
//!   each recorded by name in [`HELD_SETTINGS_STATE`] before it moves, and back after the
//!   snapshot is restored.
//!
//! The readers here see the installed files whatever phase a run is in: each move is a
//! rename, every record is written before its move and dropped after its undo, and a
//! listing that a run's state changed under is taken again.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Where a probe run keeps the installed mods, beside the install's `data/`.
pub const HELD_MODS: &str = "_bb_probe_held_mods";
/// The `--with-mod` copies a probe run has put into `data/` (JSON: `[{name, sha256}]`),
/// beside `data/`.
pub const WITH_MODS_STATE: &str = "_bb_probe_with_mods.json";
/// The probe harness's own mod, in `data/` while a run is up.
const HARNESS_MOD: &str = "mod_zz_bb_probe";
/// A probe run's snapshot of `savegames/`, beside it.
pub const SAVES_SNAPSHOT: &str = "savegames.bb_probe_backup";
/// In [`SAVES_SNAPSHOT`] once the snapshot is complete.
const SNAPSHOT_STAMP: &str = ".bb_probe_snapshot";
/// Where a probe run keeps the installed mod storages, beside `savegames/`.
pub const HELD_SETTINGS: &str = "_bb_probe_held_settings";
/// The mod storages in [`HELD_SETTINGS`] (JSON: `[{name, sha256}]`), beside it.
pub const HELD_SETTINGS_STATE: &str = "_bb_probe_held_settings.json";

/// How often a reading is taken again while a run's state changes under it, and the
/// pause between: a run's moves take milliseconds.
const ATTEMPTS: usize = 40;
const PAUSE: Duration = Duration::from_millis(50);

/// The installed mods in the data folder `dir`, unordered: its `.zip`s and those a probe
/// run holds aside ([`HELD_MODS`]), without the run's own (its harness and its
/// `--with-mod` copies, also one named like a held installed mod).
pub fn user_mods(dir: &Path) -> io::Result<Vec<PathBuf>> {
	let held = dir.parent().map(|game| game.join(HELD_MODS));
	let held_zips = || held.as_deref().and_then(|h| zips_in(h).ok()).unwrap_or_default();
	for attempt in 1.. {
		let copies = with_mod_copies(dir)?;
		// Held, data, held: a zip moved either way while listing is seen in one of them.
		let mut zips = held_zips();
		let in_data = zips_in(dir);
		zips.extend(held_zips());
		let in_data = in_data?;
		if with_mod_copies(dir)? != copies && attempt < ATTEMPTS {
			std::thread::sleep(PAUSE);
			continue;
		}
		let mut mods: BTreeMap<String, PathBuf> = BTreeMap::new();
		for zip in in_data {
			let name = file_name(&zip);
			if !copies.contains(&name) {
				mods.insert(name, zip);
			}
		}
		for zip in zips {
			mods.entry(file_name(&zip)).or_insert(zip);
		}
		return Ok(mods.into_values().collect());
	}
	unreachable!()
}

/// Where a mod [`user_mods`] listed is now: `path`, or the same zip moved by a probe run
/// since (into [`HELD_MODS`], or back into `data/`). A listed `data/` path whose name a
/// run's `--with-mod` copy now has is the held installed zip. An opened archive
/// survives the move; this is for opening one by its listed path.
pub fn locate_mod(path: &Path) -> PathBuf {
	let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
		return path.to_path_buf();
	};
	let in_held = dir.file_name().is_some_and(|n| n == HELD_MODS);
	let moved = |to: &str| dir.parent().map(|game| game.join(to).join(name)).filter(|p| p.exists());
	if !in_held && with_mod_copies(dir).is_ok_and(|c| c.contains(&name.to_string_lossy().into_owned())) {
		return moved(HELD_MODS).unwrap_or_else(|| path.to_path_buf());
	}
	if path.exists() {
		return path.to_path_buf();
	}
	moved(if in_held { "data" } else { HELD_MODS }).unwrap_or_else(|| path.to_path_buf())
}

/// The names of the `--with-mod` copies a probe run has in the data folder `dir` (or is
/// about to copy there, or has just removed): none outside a run.
fn with_mod_copies(dir: &Path) -> io::Result<Vec<String>> {
	let Some(state) = dir.parent().map(|game| game.join(WITH_MODS_STATE)) else {
		return Ok(Vec::new());
	};
	recorded_names(&state)
}

/// The `.zip`s in `dir` but the probe harness's.
fn zips_in(dir: &Path) -> io::Result<Vec<PathBuf>> {
	Ok(std::fs::read_dir(dir)?
		.flatten()
		.map(|e| e.path())
		.filter(|p| p.file_name().is_some_and(|n| {
			let n = n.to_string_lossy();
			n.ends_with(".zip") && !n.starts_with(HARNESS_MOD)
		}))
		.collect())
}

fn file_name(path: &Path) -> String {
	path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Copies the installed mod storages (`<mod>#<name>.sav`, e.g. MSU's `MSU#mod_msu#ModSettings.sav`)
/// from the original's documents folder `documents` into `dest`, as they are outside
/// probe runs: during a run, those it holds aside ([`HELD_SETTINGS`]) and those of its
/// snapshot ([`SAVES_SNAPSHOT`]); not what the run's mods write. Returns their names.
pub fn copy_user_mod_storages(documents: &Path, dest: &Path) -> io::Result<Vec<String>> {
	for attempt in 1.. {
		let storages = user_mod_storages(documents)?;
		let copied: io::Result<()> = storages.iter().try_for_each(|(name, from)| std::fs::copy(from, dest.join(name)).map(drop));
		match copied {
			Ok(()) => return Ok(storages.into_keys().collect()),
			// Moved by a run meanwhile: look again.
			Err(e) if e.kind() == io::ErrorKind::NotFound && attempt < ATTEMPTS => std::thread::sleep(PAUSE),
			Err(e) => return Err(e),
		}
	}
	unreachable!()
}

/// The installed mod storages by name, where each is now (see [`copy_user_mod_storages`]).
fn user_mod_storages(documents: &Path) -> io::Result<BTreeMap<String, PathBuf>> {
	let saves = documents.join("savegames");
	let (held, snapshot) = (documents.join(HELD_SETTINGS), documents.join(SAVES_SNAPSHOT));
	let mut out = BTreeMap::new();
	// Held aside: each recorded before it moves, so it's either held or still in savegames/.
	for name in recorded_names(&documents.join(HELD_SETTINGS_STATE))? {
		if let Some(path) = [held.join(&name), saves.join(&name)].into_iter().find(|p| p.is_file()) {
			out.insert(name, path);
		}
	}
	// The rest: a complete snapshot has the installed ones (savegames/ then has the run's
	// writes); without one, the run has not started the game, or has put savegames/ back.
	let rest = if snapshot.join(SNAPSHOT_STAMP).is_file() { snapshot } else { saves };
	let entries = match std::fs::read_dir(&rest) {
		Ok(entries) => entries,
		Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(out),
		Err(e) => return Err(e),
	};
	for path in entries.flatten().map(|e| e.path()) {
		let name = file_name(&path);
		if is_mod_storage(&name) && path.is_file() {
			out.entry(name).or_insert(path);
		}
	}
	Ok(out)
}

/// A mod's storage in savegames/ (`<mod>#<name>.sav`), not a campaign save or a probe's.
fn is_mod_storage(name: &str) -> bool {
	name.contains('#') && name.ends_with(".sav") && !name.starts_with("bb_probe_")
}

/// The `name`s a probe run's state file records (`[{"name": …, "sha256": …}, …]`, as
/// Python's `json` writes it), or none when there's no such file.
fn recorded_names(state: &Path) -> io::Result<Vec<String>> {
	let text = match std::fs::read_to_string(state) {
		Ok(text) => text,
		Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
		Err(e) => return Err(e),
	};
	json_names(&text).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("{}: not a probe run's state", state.display())))
}

/// The string values of the `"name"` keys in a JSON document (the keys and strings are
/// tokenized, so a value that reads `"name"` is no key).
fn json_names(text: &str) -> Option<Vec<String>> {
	let mut names = Vec::new();
	let mut chars = text.chars().peekable();
	let mut strings: Vec<String> = Vec::new();
	let mut key_pending = false;
	while let Some(c) = chars.next() {
		match c {
			'"' => {
				let s = json_string(&mut chars)?;
				if key_pending {
					names.push(s.clone());
					key_pending = false;
				}
				strings.push(s);
			}
			':' => key_pending = strings.last().is_some_and(|k| k == "name"),
			',' | '{' | '}' | '[' | ']' => {
				key_pending = false;
				strings.clear();
			}
			_ => {}
		}
	}
	Some(names)
}

/// The rest of a JSON string whose opening quote was read.
fn json_string(chars: &mut std::iter::Peekable<std::str::Chars>) -> Option<String> {
	let mut s = String::new();
	let hex4 = |chars: &mut std::iter::Peekable<std::str::Chars>| -> Option<u32> {
		let digits: String = (0..4).map(|_| chars.next()).collect::<Option<_>>()?;
		u32::from_str_radix(&digits, 16).ok()
	};
	loop {
		match chars.next()? {
			'"' => return Some(s),
			'\\' => match chars.next()? {
				'n' => s.push('\n'),
				't' => s.push('\t'),
				'r' => s.push('\r'),
				'b' => s.push('\u{8}'),
				'f' => s.push('\u{c}'),
				'u' => {
					let mut code = hex4(chars)?;
					if (0xd800..0xdc00).contains(&code) && chars.next() == Some('\\') && chars.next() == Some('u') {
						code = 0x10000 + ((code - 0xd800) << 10) + (hex4(chars)? - 0xdc00);
					}
					s.push(char::from_u32(code)?);
				}
				other => s.push(other),
			},
			c => s.push(c),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A fresh game folder (`data/` beside it) for one test.
	fn game(test: &str) -> PathBuf {
		let game = std::env::temp_dir().join(format!("bb-probe-aside-{test}-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&game);
		std::fs::create_dir_all(game.join("data")).unwrap();
		game
	}

	fn names(paths: &[PathBuf]) -> Vec<String> {
		let mut names: Vec<String> = paths.iter().map(|p| file_name(p)).collect();
		names.sort();
		names
	}

	#[test]
	fn the_state_files_names_read() {
		let text = "[\n {\n  \"name\": \"mod_msu 479 1.9.0 \\u00e9\\\"x\\ud83d\\ude00.zip\",\n  \"sha256\": \"name\"\n }\n]";
		assert_eq!(json_names(text).unwrap(), ["mod_msu 479 1.9.0 \u{e9}\"x\u{1f600}.zip"]);
		assert_eq!(json_names("[]").unwrap(), Vec::<String>::new());
		assert!(json_names("[{\"name\": \"cut").is_none());
	}

	#[test]
	fn a_runs_copies_are_not_the_users_mods_and_its_held_mods_are() {
		let game = game("mods");
		let (data, held) = (game.join("data"), game.join(HELD_MODS));
		std::fs::create_dir_all(&held).unwrap();
		// A --with-mod run: MSU and a_mod held, Legends and a dev build named like MSU copied in.
		for (d, name) in [
			(&data, "mod_modern_hooks.zip"),
			(&data, "mod_zz_bb_probe.zip"),
			(&data, "mod_legends.zip"),
			(&data, "mod_msu.zip"),
			(&held, "mod_msu.zip"),
			(&held, "a_mod.zip"),
		] {
			std::fs::write(d.join(name), name).unwrap();
		}
		let state = r#"[{"name": "mod_legends.zip", "sha256": "0"}, {"name": "mod_msu.zip", "sha256": "1"}]"#;
		std::fs::write(game.join(WITH_MODS_STATE), state).unwrap();
		let mods = user_mods(&data).unwrap();
		assert_eq!(names(&mods), ["a_mod.zip", "mod_modern_hooks.zip", "mod_msu.zip"]);
		let msu = mods.iter().find(|p| p.ends_with("mod_msu.zip")).unwrap();
		assert_eq!(msu, &held.join("mod_msu.zip"));
		// Listed before the run: MSU's data/ path is the held zip while the copy is there.
		assert_eq!(locate_mod(&data.join("mod_msu.zip")), held.join("mod_msu.zip"));
		// The run ends: copies gone, mods back.
		std::fs::remove_file(game.join(WITH_MODS_STATE)).unwrap();
		std::fs::remove_file(data.join("mod_legends.zip")).unwrap();
		std::fs::remove_file(data.join("mod_msu.zip")).unwrap();
		std::fs::remove_file(data.join("mod_zz_bb_probe.zip")).unwrap();
		std::fs::rename(held.join("mod_msu.zip"), data.join("mod_msu.zip")).unwrap();
		assert_eq!(locate_mod(msu), data.join("mod_msu.zip"));
		std::fs::rename(held.join("a_mod.zip"), data.join("a_mod.zip")).unwrap();
		std::fs::remove_dir(&held).unwrap();
		assert_eq!(names(&user_mods(&data).unwrap()), ["a_mod.zip", "mod_modern_hooks.zip", "mod_msu.zip"]);
		std::fs::remove_dir_all(&game).unwrap();
	}

	#[test]
	fn the_users_mod_storages_are_read_whatever_a_run_holds() {
		let docs = game("storages");
		let (saves, held, snapshot) = (docs.join("savegames"), docs.join(HELD_SETTINGS), docs.join(SAVES_SNAPSHOT));
		std::fs::create_dir_all(&saves).unwrap();
		let dest = docs.join("dest");
		// The storages copied: (name, content).
		let copy = || -> Vec<(String, String)> {
			let _ = std::fs::remove_dir_all(&dest);
			std::fs::create_dir_all(&dest).unwrap();
			let names = copy_user_mod_storages(&docs, &dest).unwrap();
			names.into_iter().map(|n| (n.clone(), std::fs::read_to_string(dest.join(&n)).unwrap())).collect()
		};
		let users = || vec![("MSU#a#S.sav".to_string(), "user a".to_string()), ("MSU#b#S.sav".to_string(), "user b".to_string())];
		std::fs::write(saves.join("MSU#a#S.sav"), "user a").unwrap();
		std::fs::write(saves.join("MSU#b#S.sav"), "user b").unwrap();
		std::fs::write(saves.join("campaign.sav"), "c").unwrap();
		std::fs::write(saves.join("bb_probe_MSU#x.sav"), "p").unwrap();
		assert_eq!(copy(), users());
		// --with-mod: a held, b recorded but not moved yet, no snapshot yet.
		std::fs::create_dir_all(&held).unwrap();
		std::fs::write(docs.join(HELD_SETTINGS_STATE), r#"[{"name": "MSU#a#S.sav", "sha256": "0"}, {"name": "MSU#b#S.sav", "sha256": "1"}]"#).unwrap();
		std::fs::rename(saves.join("MSU#a#S.sav"), held.join("MSU#a#S.sav")).unwrap();
		assert_eq!(copy(), users());
		std::fs::rename(saves.join("MSU#b#S.sav"), held.join("MSU#b#S.sav")).unwrap();
		// The snapshot (without the held ones), then the run's mods write theirs.
		std::fs::create_dir_all(&snapshot).unwrap();
		std::fs::write(snapshot.join("campaign.sav"), "c").unwrap();
		std::fs::write(snapshot.join(SNAPSHOT_STAMP), "0").unwrap();
		std::fs::write(saves.join("MSU#a#S.sav"), "run a").unwrap();
		std::fs::write(saves.join("MSU#c#S.sav"), "run c").unwrap();
		assert_eq!(copy(), users());
		// --keep-mods: nothing held, the snapshot has the installed ones.
		std::fs::remove_file(docs.join(HELD_SETTINGS_STATE)).unwrap();
		std::fs::rename(held.join("MSU#a#S.sav"), snapshot.join("MSU#a#S.sav")).unwrap();
		std::fs::rename(held.join("MSU#b#S.sav"), snapshot.join("MSU#b#S.sav")).unwrap();
		assert_eq!(copy(), users());
		std::fs::remove_dir_all(&docs).unwrap();
	}
}
