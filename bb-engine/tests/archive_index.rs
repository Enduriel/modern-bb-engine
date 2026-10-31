//! Our archive index reads every archive as the `zip` crate does: vanilla's `.dat`s and
//! every mod zip of the reference mod corpus (`bb_engine::mod_corpus`), the same names
//! in the same order, the same CRC and size per entry, the same bytes (one entry in 25,
//! and every script and brush), and an entry read in place matching its whole read.
//! Skips without the Steam install or the corpus.

use std::io::Read;
use std::path::Path;

use bb_engine::harness;
use bb_engine::vfs::DatArchive;

#[test]
fn reads_every_corpus_archive_as_the_zip_crate_does() {
	let data = Path::new(harness::STEAM_DATA_DIR);
	if !data.exists() {
		bb_engine::harness::skip(format!("{} not present", data.display()));
		return;
	}
	let corpus = match bb_engine::mod_corpus::reference() {
		Ok(corpus) => corpus,
		Err(bb_engine::mod_corpus::CorpusError::Absent(why)) => return bb_engine::harness::skip(why),
		Err(e) => panic!("{e}"),
	};
	let mut archives = harness::data_archives(data).unwrap();
	archives.extend(corpus.mods().iter().cloned());
	let (mut entries, mut compared) = (0, 0);
	for path in &archives {
		let mut ours = DatArchive::open(path).unwrap();
		let mut theirs = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
		let names: Vec<String> = theirs.file_names().map(str::to_owned).collect();
		assert_eq!(ours.names(), names, "{}: the names, in order", path.display());
		for (i, name) in names.iter().enumerate() {
			entries += 1;
			let Ok(mut file) = theirs.by_name(name) else {
				assert_eq!(ours.entry_meta(name), None, "{name}: unreadable to both");
				continue;
			};
			assert_eq!(ours.entry_meta(name), Some((file.crc32(), file.size())), "{}: {name}", path.display());
			let wanted = i % 25 == 0 || name.ends_with(".cnut") || name.ends_with(".nut") || name.ends_with(".brush");
			if !wanted || file.is_dir() {
				continue;
			}
			let mut bytes = Vec::new();
			file.read_to_end(&mut bytes).unwrap();
			assert_eq!(ours.read(name).as_deref(), Some(&bytes[..]), "{}: {name}'s bytes", path.display());
			if let Some(mut window) = ours.stream(name) {
				let mut in_place = Vec::new();
				window.read_to_end(&mut in_place).unwrap();
				assert_eq!(in_place, bytes, "{}: {name} read in place", path.display());
			}
			compared += 1;
		}
	}
	eprintln!("{} archives, {entries} entries, {compared} read and compared", archives.len());
}
