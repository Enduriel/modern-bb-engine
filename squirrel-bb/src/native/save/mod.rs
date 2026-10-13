//! Saving and loading a campaign, in this engine's own format: `World.save`,
//! `World.load`, the four save callbacks the world state registers, and the
//! `PersistenceManager` the save/load menus use.
//!
//! A save runs like the original's (load order probed 2026-10-13):
//!
//! * save: `onBeforeSave(out)` fills the metadata; each engine part writes its state
//!   (world clock, virtual time, map, world entities, rosters), entities and units each
//!   followed by their script `onSerialize(out)`; then `onSave(out)` writes the
//!   managers' state.
//! * load: `onBeforeLoad(in)` clears the managers; each part restores its state,
//!   re-creating entities and units with their saved ids (`create`, then `onInit`,
//!   which re-registers them with the managers, then their saved sprites, then
//!   `onDeserialize(in)`); then `onLoad(in)` restores the managers, which look entities
//!   up by id.
//!
//! The streams interleave engine and script data, so every part reads back exactly what
//! it wrote, in order. Keeping the engine parts behind [`SavePart`] leaves room for a
//! later importer of the original's save files.

mod storage;
pub(crate) mod stream;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use squirrel::{SQVM, SqArray, SqStrongRef, SqTable};

use storage::Storage;
use stream::{Meta, SaveReader, SaveWriter, StreamClasses};

/// What a part writes with: the engine-side writer and the script stream over it (to
/// hand to `onSerialize`).
pub(crate) struct SaveCx<'a> {
	pub vm: &'a SQVM,
	pub writer: Rc<RefCell<SaveWriter>>,
	pub stream: SqStrongRef,
}

/// What a part reads with: the engine-side reader and the script stream over it.
pub(crate) struct LoadCx<'a> {
	pub vm: &'a SQVM,
	pub reader: Rc<RefCell<SaveReader>>,
	pub stream: SqStrongRef,
}

/// One engine subsystem's share of a save. `load` gets back exactly what `save` wrote.
pub(crate) trait SavePart {
	fn save(&self, cx: &SaveCx) -> Result<(), String>;
	fn load(&self, cx: &LoadCx) -> Result<(), String>;
}

/// The callbacks `World.setOn*Callback` registers.
#[derive(Default)]
struct Callbacks {
	before_save: Option<SqStrongRef>,
	save: Option<SqStrongRef>,
	before_load: Option<SqStrongRef>,
	load: Option<SqStrongRef>,
}

struct Inner {
	storage: RefCell<Storage>,
	parts: RefCell<Vec<Rc<dyn SavePart>>>,
	callbacks: RefCell<Callbacks>,
	classes: RefCell<Option<Rc<StreamClasses>>>,
}

/// The save system. Host-owned: [`clear`](Self::clear) it before the VM closes.
pub struct SaveSystem {
	inner: Rc<Inner>,
}

impl Default for SaveSystem {
	fn default() -> Self {
		Self::new()
	}
}

/// Call a registered callback with the stream, if one is registered.
fn run_callback(vm: &SQVM, callback: Option<SqStrongRef>, stream: &SqStrongRef) -> Result<(), String> {
	callback.map_or(Ok(()), |c| c.call(vm, (stream.clone(),)))
}

fn now_secs() -> i64 {
	std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.map_or(0, |d| d.as_secs() as i64)
}

impl SaveSystem {
	/// A save system keeping saves in memory until [`set_directory`](Self::set_directory).
	pub fn new() -> Self {
		SaveSystem {
			inner: Rc::new(Inner {
				storage: RefCell::new(Storage::Memory(RefCell::default())),
				parts: RefCell::default(),
				callbacks: RefCell::default(),
				classes: RefCell::default(),
			}),
		}
	}

	/// Keep saves as `<name>.sav` files in `dir`.
	pub fn set_directory(&self, dir: PathBuf) {
		*self.inner.storage.borrow_mut() = Storage::Dir(dir);
	}

	/// Add an engine part; parts save and load in the order they were added.
	pub(crate) fn add_part(&self, part: Rc<dyn SavePart>) {
		self.inner.parts.borrow_mut().push(part);
	}

	/// Install `World.save`/`load`, the `World.setOn*Callback` registrations and the
	/// `PersistenceManager` global. Call after the `World` table exists.
	pub fn install(&self, vm: &mut SQVM) {
		let classes = Rc::new(StreamClasses::build(vm));
		*self.inner.classes.borrow_mut() = Some(classes.clone());

		macro_rules! setter {
			($field:ident) => {{
				let inner = self.inner.clone();
				vm.make_closure(move |callback: Option<SqStrongRef>| {
					inner.callbacks.borrow_mut().$field = callback;
				})
			}};
		}
		let set_before_save = setter!(before_save);
		let set_save = setter!(save);
		let set_before_load = setter!(before_load);
		let set_load = setter!(load);

		let save = {
			let inner = self.inner.clone();
			let classes = classes.clone();
			vm.make_closure(move |vm: &SQVM, file: String, label: Option<String>| -> Result<(), String> {
				let meta = Rc::new(RefCell::new(Meta {
					name: label.unwrap_or_else(|| file.clone()),
					file_name: file.clone(),
					created: now_secs(),
					..Meta::default()
				}));
				let writer = Rc::new(RefCell::new(SaveWriter::default()));
				let stream = classes.out_stream(vm, writer.clone(), meta.clone());
				let before = inner.callbacks.borrow().before_save.clone();
				run_callback(vm, before, &stream)?;
				let parts = inner.parts.borrow().clone();
				let cx = SaveCx {
					vm,
					writer: writer.clone(),
					stream: stream.clone(),
				};
				for part in parts {
					part.save(&cx)?;
				}
				let after = inner.callbacks.borrow().save.clone();
				run_callback(vm, after, &stream)?;
				let bytes = storage::encode(&meta.borrow(), &writer.borrow().buf);
				inner.storage.borrow().write(&file, bytes)
			})
		};
		let load = {
			let inner = self.inner.clone();
			let classes = classes.clone();
			vm.make_closure(move |vm: &SQVM, file: String| -> Result<(), String> {
				let bytes = inner.storage.borrow().read(&file)?;
				let (meta, reader) = storage::decode(bytes)?;
				let reader = Rc::new(RefCell::new(reader));
				let stream = classes.in_stream(vm, reader.clone(), Rc::new(RefCell::new(meta)));
				let before = inner.callbacks.borrow().before_load.clone();
				run_callback(vm, before, &stream)?;
				let parts = inner.parts.borrow().clone();
				let cx = LoadCx {
					vm,
					reader,
					stream: stream.clone(),
				};
				for part in parts {
					part.load(&cx)?;
				}
				let after = inner.callbacks.borrow().load.clone();
				run_callback(vm, after, &stream)
			})
		};

		// PersistenceManager: the saves' metadata for the menus, and deleting a save.
		let query_storages = {
			let inner = self.inner.clone();
			let classes = classes.clone();
			vm.make_closure(move |vm: &SQVM| -> SqStrongRef {
				let mut list = SqArray::new(vm);
				let storage = inner.storage.borrow();
				for name in storage.names() {
					let Ok((meta, _)) = storage.read(&name).and_then(storage::decode) else {
						continue;
					};
					list.push(classes.meta_object(vm, Rc::new(RefCell::new(meta))));
				}
				list.to_strong()
			})
		};
		let delete_storage = {
			let inner = self.inner.clone();
			vm.make_closure(move |name: String| inner.storage.borrow().delete(&name))
		};

		let mut world = SqTable::root(vm)
			.get::<SqTable>("World")
			.expect("World table must be installed before SaveSystem");
		world.set("setOnBeforeSaveCallback", set_before_save);
		world.set("setOnSaveCallback", set_save);
		world.set("setOnBeforeLoadCallback", set_before_load);
		world.set("setOnLoadCallback", set_load);
		world.set("save", save);
		world.set("load", load);
		let mut persistence = SqTable::new(vm);
		persistence.set("queryStorages", query_storages);
		persistence.set("deleteStorage", delete_storage);
		SqTable::root(vm).set("PersistenceManager", persistence);
	}

	/// The stored bytes of save `name` (for tests and tools).
	pub fn read_save(&self, name: &str) -> Result<Vec<u8>, String> {
		self.inner.storage.borrow().read(name)
	}

	/// Release every retained script reference. Call before the VM is closed.
	pub fn clear(&self) {
		*self.inner.callbacks.borrow_mut() = Callbacks::default();
		*self.inner.classes.borrow_mut() = None;
		self.inner.parts.borrow_mut().clear();
	}
}
