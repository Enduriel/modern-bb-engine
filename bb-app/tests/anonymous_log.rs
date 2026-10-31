//! log.html as bb writes it (`bb_app::logs`, in a folder of the test's own, as `--logs`
//! gives) has no path with the user's home folder in it: the engine's lines, the `log`
//! crate's warnings and the session's critical end all show it as its placeholder.

use std::path::{MAIN_SEPARATOR, Path};

use squirrel_bb::html_log::{self, Class, anonymise::Style};

#[test]
fn the_log_shows_no_home_folder() {
	let home = std::env::home_dir().expect("a home folder");
	let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("anonymous-log-{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&dir);
	let logs = bb_app::logs::Logs::start(&dir).unwrap();
	let saves = home.join("Documents").join("Battle Brothers").join("savegames");
	html_log::line(Class::Info, "Saves", format!("saves: {}", saves.display()));
	log::warn!("a library's warning about {}", home.join(".config").display());
	logs.close(Some(&format!("could not read {}", home.join("x.sav").display())));

	let text = std::fs::read_to_string(dir.join("log.html")).unwrap();
	let home = home.display().to_string();
	assert!(!text.contains(&home), "{home} in {text}");
	let placeholder = Style::HOST.home_placeholder();
	let s = MAIN_SEPARATOR;
	for shown in [
		format!("saves: {placeholder}{s}Documents{s}Battle Brothers{s}savegames"),
		format!("a library's warning about {placeholder}{s}.config"),
		format!("could not read {placeholder}{s}x.sav"),
	] {
		assert!(text.contains(&shown), "{shown:?} not in {text}");
	}
	std::fs::remove_dir_all(&dir).unwrap();
}
