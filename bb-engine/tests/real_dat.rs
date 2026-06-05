//! Validates `.dat` reading against the real installed game files, if present.
//! Skips cleanly when the Steam install isn't there (CI / other machines).

use bb_engine::DatArchive;

const DATA_001: &str =
	concat!(env!("HOME"), "/.local/share/Steam/steamapps/common/Battle Brothers/data/data_001.dat");

#[test]
fn reads_a_real_cnut_and_confirms_the_bytecode_header() {
	if !std::path::Path::new(DATA_001).exists() {
		eprintln!("skipping: {DATA_001} not present");
		return;
	}

	let mut archive = DatArchive::open(DATA_001).expect("open data_001.dat");
	assert!(archive.len() > 1000, "data_001 has thousands of entries");

	let cnut = archive
		.read("scripts/ai/tactical/agent.cnut")
		.expect("read agent.cnut");

	// The compiled-closure stream header is plaintext (small reads aren't
	// encrypted): SQ_BYTECODE_STREAM_TAG (0xFAFA) then the "RIQS" head.
	assert_eq!(&cnut[0..2], &[0xFA, 0xFA], "SQ_BYTECODE_STREAM_TAG");
	assert_eq!(&cnut[2..6], b"RIQS", "closure stream head");
	eprintln!(
		"agent.cnut: {} bytes; header + next markers = {:02x?}",
		cnut.len(),
		&cnut[6..18]
	);
}
