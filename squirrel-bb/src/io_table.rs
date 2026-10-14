//! The `IO` table: the class-name hash registry (`scriptFilenameByHash` /
//! `scriptHashByFilename`) and `enumerateFiles`, over the module system's state.

use squirrel::{SQVM, SqArray, SqStrongRef, SqTable};

use crate::hash::{class_name_hash, normalize_path};
use crate::source::ScriptSource;
use crate::state::SharedState;

/// Build the `IO` table and slot it into the root.
pub(crate) fn install<S: ScriptSource + 'static>(vm: &mut SQVM, state: &SharedState<S>) {
	let by_hash = {
		let state = state.clone();
		vm.make_closure(move |hash: i64| -> Option<String> {
			state.borrow().by_hash.get(&hash).cloned()
		})
	};
	let by_name = {
		let state = state.clone();
		vm.make_closure(move |path: &str| -> Option<i64> {
			// Like the original: only an existing script, named by its exact
			// canonical path, has a hash; anything else (wrong case, a `.nut`
			// suffix, an unknown path) yields null. Remember the mapping so a later
			// `scriptFilenameByHash` can reverse it even for an unloaded script.
			if path != normalize_path(path) {
				return None;
			}
			let known = state.borrow().prototypes.contains_key(path)
				|| state.borrow().source.load(path).is_some();
			if !known {
				return None;
			}
			let hash = class_name_hash(path);
			state
				.borrow_mut()
				.by_hash
				.entry(hash)
				.or_insert_with(|| path.to_string());
			Some(hash)
		})
	};
	let enumerate = {
		let state = state.clone();
		vm.make_closure(move |vm: &SQVM, prefix: Option<String>| -> Option<SqStrongRef> {
			// `None` (a missing directory) becomes Squirrel `null`, which
			// `preload.nut`'s DLC sweep guards against; so does a `null` directory
			// (probed). Otherwise an array of the files there, without extensions.
			let paths = state.borrow().source.list_prefix(&prefix?)?;
			let mut files = SqArray::new(vm);
			for path in paths {
				files.push(path);
			}
			Some(files.to_strong())
		})
	};

	let mut io = SqTable::new(vm);
	io.set("scriptFilenameByHash", by_hash);
	io.set("scriptHashByFilename", by_name);
	io.set("enumerateFiles", enumerate);
	SqTable::root(vm).set("IO", io);
}
