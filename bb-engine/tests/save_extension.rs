//! Campaigns save in the original's format by default, with this engine's extension
//! after the fog of war. A section's owner gets its bytes back on
//! load, or `None` from a save without them (as after the original saved the campaign
//! again: it drops the extension). A section nothing here owns rides along unchanged to
//! the next save. A campaign with no sections is the original's save, nothing after the
//! fog.
//!
//! Skips without the Steam install.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use bb_engine::harness;
use squirrel::SQVM;
use squirrel_bb::ExtensionSection;

/// A folder of its own for the test's saves, removed again by `Drop`.
struct Folder(PathBuf);

impl Drop for Folder {
	fn drop(&mut self) {
		let _ = std::fs::remove_dir_all(&self.0);
	}
}

/// A section that saves fixed bytes and records what each load handed it.
struct Recorder {
	bytes: Option<Vec<u8>>,
	loads: RefCell<Vec<Option<Vec<u8>>>>,
}

impl ExtensionSection for Recorder {
	fn name(&self) -> &str {
		"test.recorder"
	}

	fn save(&self, _: &SQVM) -> Result<Option<Vec<u8>>, String> {
		Ok(self.bytes.clone())
	}

	fn load(&self, _: &SQVM, bytes: Option<&[u8]>) -> Result<(), String> {
		self.loads.borrow_mut().push(bytes.map(<[u8]>::to_vec));
		Ok(())
	}
}

/// The extension holding `sections`, written out by hand from the documented layout.
fn extension(sections: &[(&str, &[u8])]) -> Vec<u8> {
	let mut out = b"BBME".to_vec();
	out.extend_from_slice(&1u32.to_le_bytes());
	out.extend_from_slice(&(sections.len() as u32).to_le_bytes());
	let index: usize = sections.iter().map(|(n, _)| 2 + n.len() + 16).sum();
	let mut offset = (12 + index) as u64;
	for (name, bytes) in sections {
		out.extend_from_slice(&(name.len() as u16).to_le_bytes());
		out.extend_from_slice(name.as_bytes());
		out.extend_from_slice(&offset.to_le_bytes());
		out.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
		offset += bytes.len() as u64;
	}
	for (_, bytes) in sections {
		out.extend_from_slice(bytes);
	}
	let len = out.len() as u64;
	out.extend_from_slice(&len.to_le_bytes());
	out.extend_from_slice(b"BBME");
	out
}

fn load(engine: &harness::GameEngine, name: &str) {
	engine
		.run_script(&format!("::World.State.m.CampaignLoadTime = -100.0; ::World.State.loadCampaign(\"{name}\");"), "load")
		.unwrap_or_else(|e| panic!("load {name}: {e}"));
}

#[test]
fn the_extension_rides_after_the_fog_and_survives_its_absence() {
	if !Path::new(harness::STEAM_DATA_DIR).exists() {
		eprintln!("skipping: no Steam install");
		return;
	}
	let folder = Folder(std::env::temp_dir().join(format!("bb-save-extension-{}", std::process::id())));
	let _ = std::fs::remove_dir_all(&folder.0);
	let file = |name: &str| folder.0.join(format!("{name}.sav"));

	let engine = harness::open_install(Path::new(harness::STEAM_DATA_DIR)).expect("open install");
	engine.saves().set_directory(folder.0.clone());
	harness::boot_menu(&engine).expect("boot the menu");
	harness::prepare_new_campaign(&engine, "rust-engine-test").expect("prepare the campaign");
	harness::start_new_campaign(&engine).expect("startNewCampaign");
	engine.run_script("::World.State.setPause(true);", "pause").expect("pause");

	// No sections: the original's save as it is.
	engine.run_script("::World.State.saveCampaign(\"plain\");", "save").expect("save");
	let plain = std::fs::read(file("plain")).expect("plain save");
	assert!(plain.starts_with(&[0xbb, 0, 2]), "the original's container by default");
	assert_ne!(&plain[plain.len() - 4..], b"BBME", "nothing after the fog");

	let recorder = Rc::new(Recorder {
		bytes: Some(b"recorded state".to_vec()),
		loads: RefCell::default(),
	});
	engine.saves().add_extension_section(recorder.clone());
	engine.run_script("::World.State.saveCampaign(\"ext\");", "save").expect("save");
	let ext = std::fs::read(file("ext")).expect("ext save");
	let tail = extension(&[("test.recorder", b"recorded state")]);
	assert!(ext.ends_with(&tail), "the extension, as documented, ends the file");
	// Before it, the fog of war ends the body, as in the plain save (0xbb, the length,
	// the 2048 x 2048 mask, `false`).
	let fog = 1 + 4 + 2048 * 2048 + 1;
	let body = &ext[..ext.len() - tail.len()];
	assert_eq!(body[body.len() - fog..], plain[plain.len() - fog..], "the fog ends the body");

	load(&engine, "ext");
	assert_eq!(recorder.loads.borrow().last(), Some(&Some(b"recorded state".to_vec())), "the section's bytes back");
	load(&engine, "plain");
	assert_eq!(recorder.loads.borrow().last(), Some(&None), "a save without the section: None");

	// A section nothing here owns (another mod's), next to ours: kept to the next save.
	let mut foreign = ext[..ext.len() - tail.len()].to_vec();
	foreign.extend_from_slice(&extension(&[("other.mod", &[1, 2, 3]), ("test.recorder", b"old")]));
	std::fs::write(file("foreign"), &foreign).expect("write");
	load(&engine, "foreign");
	assert_eq!(recorder.loads.borrow().last(), Some(&Some(b"old".to_vec())));
	engine.run_script("::World.State.saveCampaign(\"again\");", "save").expect("save");
	let again = std::fs::read(file("again")).expect("again save");
	assert!(
		again.ends_with(&extension(&[("other.mod", &[1, 2, 3]), ("test.recorder", b"recorded state")])),
		"the foreign section is written back; ours is saved anew"
	);

	// Bytes after the fog that aren't an extension: the original loads such a save, and
	// so does this engine (the bytes are ignored).
	let mut junk = plain.clone();
	junk.extend_from_slice(&[0x5a; 1000]);
	std::fs::write(file("junk"), &junk).expect("write");
	load(&engine, "junk");
	assert_eq!(recorder.loads.borrow().last(), Some(&None));
}
