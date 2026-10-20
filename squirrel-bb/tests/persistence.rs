//! `PersistenceManager`'s storages, as the original behaves (probed 2026-10-14) and as
//! MSU uses them for its persistent data: create, write between `beginWrite` and
//! `endWrite`, list, load and read back, delete. Plus `IO.enumerateFiles`' argument
//! handling. Storages stay in memory here (no save directory).

use squirrel_bb::{Engine, LoadedScript, ScriptSource};

struct NoScripts;
impl ScriptSource for NoScripts {
	fn load(&self, _path: &str) -> Option<LoadedScript> {
		None
	}
	fn list_prefix(&self, _prefix: &str) -> Option<Vec<String>> {
		None
	}
}

#[test]
fn storages_round_trip_like_the_original() {
	let engine = Engine::new(NoScripts);
	engine
		.run_script(
			r#"
			local w = ::PersistenceManager.createStorage("MSU#mod_test#settings");
			w.beginWrite();
			w.writeU8(200); w.writeString("hello"); w.writeU32(70000); w.writeI32(-5);
			w.writeF32(1.5); w.writeBool(true); w.writeU16(65535);
			w.endWrite();

			local listed = null;
			foreach (st in ::PersistenceManager.queryStorages())
				if (st.getFileName() == "MSU#mod_test#settings") listed = st;
			assert(listed != null);
			assert(listed.getName() == "MSU#mod_test#settings");
			assert(listed.getVersion() == 0);
			assert(listed.getInt("x") == 0 && listed.getString("x") == "");
			assert(typeof listed.getCreationDate() == "string" && listed.getCreationDate().len() == 17);

			local r = ::PersistenceManager.loadStorage("MSU#mod_test#settings");
			r.beginRead();
			assert(r.readU8() == 200);
			assert(r.readString() == "hello");
			assert(r.readU32() == 70000);
			assert(r.readI32() == -5);
			assert(r.readF32() == 1.5);
			assert(r.readBool() == true);
			assert(r.readU16() == 65535);
			r.endRead();
			assert(::PersistenceManager.loadStorage("missing") == null);

			::PersistenceManager.deleteStorage("MSU#mod_test#settings");
			assert(::PersistenceManager.queryStorages().len() == 0);

			// deleteStorages() removes every storage.
			foreach (n in ["a", "b"]) { local s = ::PersistenceManager.createStorage(n); s.beginWrite(); s.endWrite(); }
			assert(::PersistenceManager.queryStorages().len() == 2);
			::PersistenceManager.deleteStorages();
			assert(::PersistenceManager.queryStorages().len() == 0);
			"#,
			"storages",
		)
		.expect("storages behave like the original's");
}

#[test]
fn enumerate_files_takes_one_argument_and_null_lists_nothing() {
	let engine = Engine::new(NoScripts);
	engine
		.run_script(r#"assert(::IO.enumerateFiles(null) == null);"#, "null dir")
		.expect("a null directory lists nothing");
	let err = engine
		.run_script(r#"::IO.enumerateFiles();"#, "no args")
		.expect_err("no directory is an error");
	assert!(err.contains("wrong number of parameters"), "{err}");
}

/// Storages are files in the original's container, so a mod's data is the same file for
/// both engines: one the original wrote reads back here, and one written here has the
/// original's header and `u16`-length strings.
#[test]
fn storages_are_the_originals_files() {
	let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("storages_are_the_originals_files");
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).unwrap();
	// A storage as the original writes it: magic, format 2, version 0, the creation and
	// modification times, the label, no metadata, the end magic, then the body:
	// writeString("hi"), writeU8(7).
	let header = |label: &[u8]| {
		let mut h = vec![0xbb, 0, 2, 0, 0, 0, 0];
		h.extend_from_slice(&1_700_000_000u64.to_le_bytes());
		h.extend_from_slice(&1_700_000_000u64.to_le_bytes());
		h.extend_from_slice(&(label.len() as u16).to_le_bytes());
		h.extend_from_slice(label);
		h.extend_from_slice(&[0, 0, 0xbb, 0]);
		h
	};
	let body = [2, 0, b'h', b'i', 7];
	std::fs::write(dir.join("MSU#x.sav"), [header(b"MSU#x"), body.to_vec()].concat()).unwrap();

	let engine = Engine::new(NoScripts);
	engine.saves().set_directory(dir.clone());
	engine
		.run_script(
			r#"
			local r = ::PersistenceManager.loadStorage("MSU#x");
			assert(r.getMetaData().getName() == "MSU#x");
			assert(r.readString() == "hi" && r.readU8() == 7);
			local w = ::PersistenceManager.createStorage("mine");
			w.beginWrite(); w.writeString("hi"); w.writeU8(7); w.endWrite();
			"#,
			"original storages",
		)
		.expect("an original storage reads back");
	let mine = std::fs::read(dir.join("mine.sav")).unwrap();
	let expected = [header(b"mine"), body.to_vec()].concat();
	assert_eq!(mine[..7], expected[..7]);
	// The times are now; the rest is as the original writes it.
	assert_eq!(mine[23..], expected[23..]);
}

/// A stream has both directions' members, as the original's one stream class: its own
/// file name either way, and the other direction refusing (the original's read garbage
/// from a stream being written, and its `beginRead` hangs).
#[test]
fn a_stream_knows_its_file_and_refuses_the_other_direction() {
	let engine = Engine::new(NoScripts);
	engine
		.run_script(
			r#"
			local w = ::PersistenceManager.createStorage("two_ways");
			assert(w.getFileName() == "two_ways");
			w.beginWrite();
			w.writeU8(7);
			local refused = null;
			try { w.readU8(); } catch (e) { refused = e; }
			assert(refused == "readU8: the stream is being written");
			w.endWrite();
			local r = ::PersistenceManager.loadStorage("two_ways");
			assert(r.getFileName() == "two_ways");
			r.beginRead();
			refused = null;
			try { r.writeU8(1); } catch (e) { refused = e; }
			assert(refused == "writeU8: the stream is being read");
			assert(r.readU8() == 7);
			r.endRead();
			::PersistenceManager.deleteStorage("two_ways");
			"#,
			"two_ways",
		)
		.expect("both directions' members");
}

/// `queryStorages` lists as the original's (disassembled, then probed against a folder
/// of 132 saves): newest modification time first, the time each file's header records
/// (not its creation time, nor the file system's time). Equal times keep the order the
/// original's PhysFS lists the folder in: by whole file name, `strcmp`, so case-sensitive
/// and with `.sav` taking part (`x-1.sav` before `x.sav`).
#[test]
fn storages_list_newest_first_then_by_file_name() {
	let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("storages_list_newest_first");
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).unwrap();
	let header = |label: &str, created: u64, modified: u64| {
		let mut h = vec![0xbb, 0, 2, 0, 0, 0, 0];
		h.extend_from_slice(&created.to_le_bytes());
		h.extend_from_slice(&modified.to_le_bytes());
		h.extend_from_slice(&(label.len() as u16).to_le_bytes());
		h.extend_from_slice(label.as_bytes());
		h.extend_from_slice(&[0, 0, 0xbb, 0]);
		h
	};
	// (name, created, modified, the file system's modification time)
	let saves = [
		("c", 200, 200, 1_900_000_000),
		("A", 500, 300, 1_800_000_000),
		("x", 100, 100, 1_700_000_000),
		("b", 100, 400, 1_600_000_000),
		("x-1", 100, 100, 1_700_000_000),
		("X", 100, 100, 1_700_000_000),
	];
	for (name, created, modified, mtime) in saves {
		let file = std::fs::File::create(dir.join(format!("{name}.sav"))).unwrap();
		std::io::Write::write_all(&mut &file, &header(name, created, modified)).unwrap();
		file.set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(mtime)).unwrap();
	}
	std::fs::write(dir.join("broken.sav"), b"not a save").unwrap();
	std::fs::write(dir.join("notes.txt"), header("notes", 900, 900)).unwrap();

	let engine = Engine::new(NoScripts);
	engine.saves().set_directory(dir);
	engine
		.run_script(
			r#"
			local s = "";
			foreach (m in ::PersistenceManager.queryStorages()) s += m.getFileName() + ",";
			if (s != "b,A,c,X,x-1,x,") throw "listed " + s;
			"#,
			"list order",
		)
		.expect("the original's order");
}
