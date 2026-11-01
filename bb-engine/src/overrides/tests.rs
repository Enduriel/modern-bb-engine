use super::*;
use crate::test_game::{Game, MID, NEW, OLD};

fn messages(game: &Game) -> Vec<String> {
	find(&game.source()).iter().map(Conflict::message).collect()
}

#[test]
fn a_file_two_mods_provide_is_reported_for_the_one_that_loses() {
	let mut game = Game::new("two-mods");
	game.zip("data_001.dat", &[("gfx/a.png", b"vanilla", MID)])
		.zip("mod_a.zip", &[("gfx/a.png", b"a", MID), ("gfx/b.png", b"a", MID), ("gfx/only_a.png", b"a", MID)])
		.zip("mod_b.zip", &[("gfx/a.png", b"b", MID), ("gfx/b.png", b"b", MID)]);
	assert_eq!(messages(&game), ["\"mod_b.zip\" overrides 2 files of \"mod_a.zip\": gfx/a.png, gfx/b.png"]);
}

#[test]
fn identical_copies_and_replacing_vanillas_files_are_not_conflicts() {
	let mut game = Game::new("benign");
	game.zip("data_001.dat", &[("gfx/a.png", b"vanilla", MID), ("scripts/x.cnut", b"compiled", OLD)])
		.zip("mod_a.zip", &[("gfx/a.png", b"same", MID), ("scripts/x.nut", b"source", MID)])
		.zip("mod_b.zip", &[("gfx/a.png", b"same", MID)]);
	assert_eq!(messages(&game), Vec::<String>::new());
}

#[test]
fn a_mods_nut_no_newer_than_vanillas_cnut_is_reported_unused() {
	let mut game = Game::new("old-nut");
	game.zip("data_001.dat", &[("scripts/x.cnut", b"compiled", MID), ("scripts/y.cnut", b"compiled", MID)])
		.zip("mod_a.zip", &[("scripts/x.nut", b"source", OLD), ("scripts/y.nut", b"source", MID)]);
	assert_eq!(
		messages(&game),
		["\"data_001.dat\" (the game's own) overrides 2 files of \"mod_a.zip\": \
		  scripts/x.nut (its .cnut is as new or newer), scripts/y.nut (its .cnut is as new or newer)"]
	);
}

#[test]
fn between_two_mods_the_newer_of_the_nut_and_cnut_wins() {
	// mod_a loads first but wins both: its .nut is newer, and its .cnut ties (a tie loads
	// the .cnut).
	let mut game = Game::new("nut-cnut");
	game.zip("mod_a.zip", &[("scripts/e.nut", b"source", NEW), ("scripts/f.cnut", b"compiled", MID)])
		.zip("mod_b.zip", &[("scripts/e.cnut", b"compiled", OLD), ("scripts/f.nut", b"source", MID)]);
	assert_eq!(
		messages(&game),
		["\"mod_a.zip\" overrides 2 files of \"mod_b.zip\": scripts/e.cnut (its .nut is newer), \
		  scripts/f.nut (its .cnut is as new or newer)"]
	);
}

#[test]
fn loose_files_in_the_data_folder_win_and_the_install_folders_are_ignored() {
	let mut game = Game::new("loose");
	game.zip("mod_a.zip", &[("scripts/a.nut", b"zip", MID), ("scripts/b.nut", b"zip", MID)])
		.loose("data/scripts/a.nut", b"loose", 2021)
		.loose("scripts/b.nut", b"loose", 2021);
	assert_eq!(messages(&game), ["\"loose files in data/\" overrides 1 file of \"mod_a.zip\": scripts/a.nut"]);
}

#[test]
fn a_mod_archive_loading_before_vanillas_is_reported_unused() {
	let mut game = Game::new("early");
	game.zip("aaa.zip", &[("gfx/a.png", b"mod", MID)]).zip("data_001.dat", &[("gfx/a.png", b"vanilla", MID)]);
	assert_eq!(messages(&game), ["\"data_001.dat\" (the game's own) overrides 1 file of \"aaa.zip\": gfx/a.png"]);
}
