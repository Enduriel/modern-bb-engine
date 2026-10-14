//! `PersistenceManager`: the named storages saves are kept in, which the save and load
//! menus list and delete, and which mods keep their own data in (MSU's persistent data,
//! as `MSU#…` storages beside the saves).
//!
//! Probed 2026-10-14: `createStorage(name)` gives a storage written between
//! `beginWrite()` and `endWrite()`, listed by `queryStorages()` under its name (no
//! label, version 0); `loadStorage(name)` gives one read between `beginRead()` and
//! `endRead()`, or null if there is none; `deleteStorage(name)` removes one.
//! `deleteStorages()` removes every storage (disassembled, `0x5671f0`).
//!
//! Storages are written in the original's container, so a mod's data is the same file
//! for both engines (MSU's settings, say), and read in either format.

use std::cell::RefCell;
use std::rc::Rc;

use squirrel::{SQVM, SqArray, SqStrongRef};

use super::stream::{Meta, SaveReader};
use super::{Inner, container, now_secs, storage};

/// The `PersistenceManager` natives, by name.
pub(super) fn natives(inner: &Rc<Inner>, vm: &mut SQVM) -> Vec<(&'static str, SqStrongRef)> {
	// The saves' metadata, for the menus (and for mods to find their storages).
	let query_storages = {
		let inner = inner.clone();
		vm.make_closure(move |vm: &SQVM| -> Result<SqStrongRef, String> {
			let classes = inner.classes.borrow().clone().ok_or("PersistenceManager: not installed")?;
			let mut list = SqArray::new(vm);
			let storage = inner.storage.borrow();
			for name in storage.names() {
				let Ok((_, meta, _)) = storage.read(&name).and_then(|b| storage::decode(b, &name)) else {
					continue;
				};
				list.push(classes.meta_object(vm, Rc::new(RefCell::new(meta))));
			}
			Ok(list.to_strong())
		})
	};
	let create_storage = {
		let inner = inner.clone();
		vm.make_closure(move |vm: &SQVM, name: String| -> Result<SqStrongRef, String> {
			let classes = inner.classes.borrow().clone().ok_or("PersistenceManager: not installed")?;
			let now = now_secs();
			let meta = Meta {
				name: name.clone(),
				file_name: name.clone(),
				created: now,
				modified: now,
				..Meta::default()
			};
			let store = inner.clone();
			let end_write = Rc::new(move |meta: &Meta, body: &[u8]| store.storage.borrow().write(&name, container::encode(meta, body)));
			Ok(classes.storage(vm, Rc::new(RefCell::new(meta)), end_write))
		})
	};
	let load_storage = {
		let inner = inner.clone();
		vm.make_closure(move |vm: &SQVM, name: String| -> Result<Option<SqStrongRef>, String> {
			let classes = inner.classes.borrow().clone().ok_or("PersistenceManager: not installed")?;
			let Ok((_, meta, reader)) = inner.storage.borrow().read(&name).and_then(|b| storage::decode(b, &name)) else {
				return Ok(None);
			};
			let reader: Rc<RefCell<SaveReader>> = Rc::new(RefCell::new(reader));
			Ok(Some(classes.in_stream(vm, reader, Rc::new(RefCell::new(meta)))))
		})
	};
	let delete_storage = {
		let inner = inner.clone();
		vm.make_closure(move |name: String| inner.storage.borrow().delete(&name))
	};
	let delete_storages = {
		let inner = inner.clone();
		vm.make_closure(move || {
			let storage = inner.storage.borrow();
			for name in storage.names() {
				storage.delete(&name);
			}
		})
	};
	vec![
		("queryStorages", query_storages),
		("createStorage", create_storage),
		("loadStorage", load_storage),
		("deleteStorage", delete_storage),
		("deleteStorages", delete_storages),
	]
}
