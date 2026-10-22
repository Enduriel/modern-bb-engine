//! Fake folder trees for the install tests, under the build's target folder (never the
//! system's temporary folder): one folder per test, emptied as it starts.

use std::path::{Path, PathBuf};

pub struct Tree {
	root: PathBuf,
}

impl Tree {
	/// The tree named `name`, empty.
	pub fn new(name: &str) -> Tree {
		// The test binary is `<target>/<profile>/deps/<name>`.
		let exe = std::env::current_exe().expect("the test binary");
		let profile = exe.parent().and_then(Path::parent).expect("the target's profile folder");
		let root = profile.join("install_trees").join(name);
		let _ = std::fs::remove_dir_all(&root);
		std::fs::create_dir_all(&root).expect("the tree's folder");
		Tree { root }
	}

	pub fn root(&self) -> &Path {
		&self.root
	}

	/// A game folder named `name` in the tree, with `data/data_001.dat`.
	pub fn game(&self, name: &str) -> PathBuf {
		let game = self.root.join(name);
		std::fs::create_dir_all(game.join("data")).expect("the game's data folder");
		std::fs::write(game.join("data").join(super::layout::MARKER), b"").expect("data_001.dat");
		game
	}

	/// A Steam folder named `name` whose one library (itself) has the game installed in
	/// `common/<install_dir>`, or only listed there with `install_dir` missing when
	/// `installed` is false.
	pub fn steam(&self, name: &str, install_dir: &str, installed: bool) -> PathBuf {
		let steam = self.root.join(name);
		let steamapps = steam.join("steamapps");
		std::fs::create_dir_all(steamapps.join("common")).expect("steamapps");
		let library = steam.display().to_string().replace('\\', "\\\\");
		let folders = format!(
			"\"libraryfolders\"\n{{\n\t\"0\"\n\t{{\n\t\t\"path\"\t\t\"{library}\"\n\t\t\"apps\"\n\t\t{{\n\t\t\t\"365360\"\t\t\"1\"\n\t\t}}\n\t}}\n}}\n"
		);
		std::fs::write(steamapps.join("libraryfolders.vdf"), folders).expect("libraryfolders.vdf");
		let manifest = format!(
			"\"AppState\"\n{{\n\t\"appid\"\t\t\"365360\"\n\t\"name\"\t\t\"Battle Brothers\"\n\t\"installdir\"\t\t\"{install_dir}\"\n}}\n"
		);
		std::fs::write(steamapps.join("appmanifest_365360.acf"), manifest).expect("the app's manifest");
		if installed {
			let data = steamapps.join("common").join(install_dir).join("data");
			std::fs::create_dir_all(&data).expect("the game's data folder");
			std::fs::write(data.join(super::layout::MARKER), b"").expect("data_001.dat");
		}
		steam
	}
}
