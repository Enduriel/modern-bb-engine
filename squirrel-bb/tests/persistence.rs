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
