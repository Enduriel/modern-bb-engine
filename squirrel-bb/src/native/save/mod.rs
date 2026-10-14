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

use std::cell::{Cell, RefCell};
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
	/// A save or load is running (the original's `World+0x2d0`): another one is
	/// ignored, and `canLoad` says no.
	busy: Cell<bool>,
	/// `World.presave()` asked for the save's picture of the world, taken the next time
	/// the world is drawn (`World+0x2cd`). Until then saves are ignored.
	presave_pending: Cell<bool>,
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

/// Write the campaign to save `file`: `onBeforeSave`, each engine part, `onSave`.
fn save_campaign(inner: &Inner, classes: &StreamClasses, vm: &SQVM, file: String, label: Option<String>) -> Result<(), String> {
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
}

/// Read the campaign back from save `file`, in the order it was written.
fn load_campaign(inner: &Inner, classes: &StreamClasses, vm: &SQVM, file: String) -> Result<(), String> {
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
				busy: Cell::new(false),
				presave_pending: Cell::new(false),
			}),
		}
	}

	/// The world's drawing takes the picture `World.presave()` asked for (the original's
	/// world scene render, which only runs while the world is shown); saves are allowed
	/// again after it.
	pub fn presave_hook(&self) -> impl Fn(&SQVM) -> Result<(), String> + 'static {
		let inner = self.inner.clone();
		move |vm: &SQVM| {
			if inner.presave_pending.get() {
				let world = SqTable::root(vm).get::<SqStrongRef>("World").ok_or("World missing")?;
				if world.call_method_ret::<bool, _>(vm, "isVisible", ())?.unwrap_or(true) {
					inner.presave_pending.set(false);
				}
			}
			Ok(())
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

		// World.save does nothing while a save or load is running, or while a presave's
		// picture is still to be taken (disassembled, 0x4f4650).
		let save = {
			let inner = self.inner.clone();
			let classes = classes.clone();
			vm.make_closure(move |vm: &SQVM, file: String, label: Option<String>| -> Result<(), String> {
				if inner.busy.get() || inner.presave_pending.get() {
					return Ok(());
				}
				inner.busy.set(true);
				let result = save_campaign(&inner, &classes, vm, file, label);
				inner.busy.set(false);
				result
			})
		};
		// World.load does nothing while a save or load is running (0x4f4ac0).
		let load = {
			let inner = self.inner.clone();
			let classes = classes.clone();
			vm.make_closure(move |vm: &SQVM, file: String| -> Result<(), String> {
				if inner.busy.get() {
					return Ok(());
				}
				inner.busy.set(true);
				let result = load_campaign(&inner, &classes, vm, file);
				inner.busy.set(false);
				result
			})
		};
		// World.presave(): take the save's picture of the world when it's next drawn
		// (ironman saves on entering a town show the world, not the town screen).
		let presave = {
			let inner = self.inner.clone();
			vm.make_closure(move || inner.presave_pending.set(true))
		};
		// World.canLoad(name): no save or load is running and the save exists (0x4f4a90).
		let can_load = {
			let inner = self.inner.clone();
			vm.make_closure(move |name: String| !inner.busy.get() && inner.storage.borrow().exists(&name))
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
		world.set("presave", presave);
		world.set("canLoad", can_load);
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
