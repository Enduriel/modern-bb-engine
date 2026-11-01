use squirrel_bb::{LoadedScript, ScriptSource};

use super::*;
use crate::test_game::{Game, MID, NEW, OLD};

/// The file `path`'s script loads from, and the label of where: "data" or the archive's
/// file name.
fn chosen(game: &Game, path: &str) -> Option<(String, String)> {
	let source = game.source();
	source.with_archives(|archives| {
		let (name, place) = source.script_file(archives, path)?;
		let at = match place {
			Place::Dir(_) => "data".to_string(),
			Place::Archive(i) => archives[i].path().file_name().unwrap().to_string_lossy().into_owned(),
		};
		Some((name, at))
	})
}

fn pair(name: &str, at: &str) -> Option<(String, String)> {
	Some((name.to_string(), at.to_string()))
}

#[test]
fn the_data_folder_comes_before_the_archives_and_the_install_folder_is_not_read() {
	let mut game = Game::new("search-order");
	game.zip("mod_a.zip", &[("scripts/a.nut", b"zip a", MID), ("scripts/b.nut", b"zip b", MID), ("x.txt", b"zip", MID)])
		.zip("mod_b.zip", &[("scripts/c.nut", b"zip b", MID)])
		.zip("mod_c.zip", &[("scripts/c.nut", b"zip c", MID)])
		.loose("data/scripts/a.nut", b"loose a", 2021)
		.loose("scripts/b.nut", b"loose b", 2021)
		.loose("data/x.txt/y", b"", 2021)
		.loose("data/y.txt", b"data", 2021)
		.loose("y.txt", b"install", 2021)
		.loose("z.txt", b"install", 2021)
		.loose("scripts/z.nut", b"install", 2021);
	assert_eq!(chosen(&game, "scripts/a"), pair("scripts/a.nut", "data"));
	assert_eq!(chosen(&game, "scripts/b"), pair("scripts/b.nut", "mod_a.zip"));
	assert_eq!(chosen(&game, "scripts/c"), pair("scripts/c.nut", "mod_c.zip"), "the last archive wins");
	let source = game.source();
	assert_eq!(source.read_file("y.txt").as_deref(), Some(&b"data"[..]));
	assert_eq!(source.read_file("z.txt"), None, "the install folder isn't read");
	assert!(source.load("scripts/z").is_none());
	assert_eq!(source.read_file("x.txt").as_deref(), Some(&b"zip"[..]), "a folder of that name is not a file");
	assert!(matches!(source.load("scripts/a"), Some(LoadedScript::Source(s)) if s == b"loose a"));
}

#[test]
fn of_a_scripts_nut_and_cnut_the_newer_wins_and_a_tie_loads_the_cnut() {
	let mut game = Game::new("nut-or-cnut");
	game.zip("data_001.dat", &[("scripts/v.cnut", b"", MID), ("scripts/w.cnut", b"", MID), ("scripts/x.cnut", b"", MID)])
		.zip("mod_a.zip", &[("scripts/e.nut", b"", NEW), ("scripts/f.cnut", b"", MID), ("scripts/x.nut", b"", NEW)])
		.zip("mod_b.zip", &[
			("scripts/e.cnut", b"", OLD),
			("scripts/f.nut", b"", MID),
			("scripts/v.nut", b"", OLD),
			("scripts/w.nut", b"", NEW),
			("scripts/x.nut", b"", OLD),
		]);
	// A newer .nut wins whichever archive comes later; a tie loads the .cnut.
	assert_eq!(chosen(&game, "scripts/e"), pair("scripts/e.nut", "mod_a.zip"));
	assert_eq!(chosen(&game, "scripts/f"), pair("scripts/f.cnut", "mod_a.zip"));
	// A mod's .nut replaces vanilla's .cnut only when newer.
	assert_eq!(chosen(&game, "scripts/v"), pair("scripts/v.cnut", "data_001.dat"));
	assert_eq!(chosen(&game, "scripts/w"), pair("scripts/w.nut", "mod_b.zip"));
	// The .nut compared is the one the search order picks (mod_b's, older), not the newest.
	assert_eq!(chosen(&game, "scripts/x"), pair("scripts/x.cnut", "data_001.dat"));
}

#[test]
fn loose_scripts_compare_by_their_file_times() {
	let mut game = Game::new("loose-times");
	game.zip("mod_a.zip", &[("scripts/f.cnut", b"", MID), ("scripts/g.nut", b"", MID)])
		.loose("data/scripts/f.nut", b"", 2025)
		.loose("data/scripts/g.cnut", b"", 2025)
		.loose("data/scripts/m.cnut", b"", 2025)
		.loose("data/scripts/m.nut", b"", 2025);
	assert_eq!(chosen(&game, "scripts/f"), pair("scripts/f.nut", "data"));
	assert_eq!(chosen(&game, "scripts/g"), pair("scripts/g.cnut", "data"));
	assert_eq!(chosen(&game, "scripts/m"), pair("scripts/m.cnut", "data"));
	assert_eq!(chosen(&game, "scripts/none"), None);
}
