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

mod container;
mod meta;
pub(crate) use meta::format_float as meta_format_float;
pub mod original;
mod persistence;
mod storage;
pub(crate) mod stream;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use squirrel::{SQVM, SqStrongRef, SqTable};

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
	/// The engine's parts for the original's layout (see [`original`]).
	original: RefCell<Option<Rc<dyn original::world::OriginalWorld>>>,
	/// The format campaigns are saved in.
	format: Cell<SaveFormat>,
	/// The game runs modified (mods are loaded): a campaign save records it, and the
	/// original warns when it loads such a save.
	modified: Cell<bool>,
	callbacks: RefCell<Callbacks>,
	classes: RefCell<Option<Rc<StreamClasses>>>,
	/// A save or load is running (the original's `World+0x2d0`): another one is
	/// ignored, and `canLoad` says no.
	busy: Cell<bool>,
	/// `World.presave()` asked for the save's picture of the world, taken the next time
	/// the world is drawn (`World+0x2cd`). Until then saves are ignored.
	presave_pending: Cell<bool>,
}

/// The format campaigns are saved in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveFormat {
	/// This engine's own.
	Own,
	/// The original's: its container and its layout, which the original game loads.
	Original,
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
	let now = now_secs();
	let meta = Rc::new(RefCell::new(Meta {
		name: label.unwrap_or_else(|| file.clone()),
		file_name: file.clone(),
		created: now,
		modified: now,
		..Meta::default()
	}));
	// As the original's World.save, before onBeforeSave (`0x4f46e5`).
	meta.borrow_mut().set_int("modified".to_string(), i64::from(inner.modified.get()));
	let mut original = inner.format.get() == SaveFormat::Original;
	if let Some(world) = inner.original.borrow().as_ref().filter(|_| original) {
		let (w, h) = world.map_size();
		if w > original::MAX_MAP.0 || h > original::MAX_MAP.1 {
			// The original's engine makes no map past its clamp, so its format can't
			// hold one: this engine's own format it is.
			let warning = format!(
				"Saving \"{file}\": a {w} x {h} world map is larger than the original's engine makes \
				 ({} x {}), so it is saved in this engine's own format",
				original::MAX_MAP.0,
				original::MAX_MAP.1
			);
			if let Some(log) = squirrel::SqTable::root(vm).get::<SqStrongRef>("logWarning") {
				log.call(vm, (warning,))?;
			}
			original = false;
		}
	}
	let writer = Rc::new(RefCell::new(SaveWriter {
		strings: if original { stream::StrLen::U16 } else { stream::StrLen::U32 },
		..SaveWriter::default()
	}));
	let stream = classes.out_stream(vm, writer.clone(), meta.clone());
	let before = inner.callbacks.borrow().before_save.clone();
	run_callback(vm, before, &stream)?;
	if original {
		let world = inner.original.borrow().clone().ok_or("saving in the original's format: no world to save")?;
		let after = inner.callbacks.borrow().save.clone();
		let cx = SaveCx { vm, writer: writer.clone(), stream: stream.clone() };
		original::export::save(world.as_ref(), &cx, after)?;
		let bytes = container::encode(&meta.borrow(), &writer.borrow().buf);
		return inner.storage.borrow().write(&file, bytes);
	}
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
	let (format, meta, reader) = storage::decode(bytes, &file)?;
	let version = meta.version;
	let reader = Rc::new(RefCell::new(reader));
	let stream = classes.in_stream(vm, reader.clone(), Rc::new(RefCell::new(meta)));
	if format == storage::Format::Original {
		let world = inner.original.borrow().clone().ok_or("loading the original's saves: no world to load into")?;
		let cx = LoadCx { vm, reader, stream };
		let (before, after) = {
			let c = inner.callbacks.borrow();
			(c.before_load.clone(), c.load.clone())
		};
		return original::import::load(world.as_ref(), &cx, version, before, after);
	}
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
				original: RefCell::default(),
				format: Cell::new(SaveFormat::Own),
				modified: Cell::new(false),
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

	/// The saves kept in memory, and their bytes (none when they go to a directory).
	pub fn memory_use(&self) -> (usize, usize) {
		let storage = self.inner.storage.borrow();
		match &*storage {
			Storage::Memory(_) => (storage.names().len(), storage.memory_bytes()),
			Storage::Dir(_) => (0, 0),
		}
	}

	/// Keep saves as `<name>.sav` files in `dir`.
	pub fn set_directory(&self, dir: PathBuf) {
		*self.inner.storage.borrow_mut() = Storage::Dir(dir);
	}

	/// Whether the game runs modified (mods are loaded), which campaign saves record.
	pub fn set_modified(&self, modified: bool) {
		self.inner.modified.set(modified);
	}

	/// Save campaigns in `format` (this engine's own unless set). Saves of either format
	/// load.
	pub fn set_format(&self, format: SaveFormat) {
		self.inner.format.set(format);
	}

	/// The engine's parts for loading (and writing) the original's layout.
	pub(crate) fn set_original_world(&self, world: Rc<dyn original::world::OriginalWorld>) {
		*self.inner.original.borrow_mut() = Some(world);
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
		let persistence_natives = persistence::natives(&self.inner, vm);

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
		for (name, f) in persistence_natives {
			persistence.set(name, f);
		}
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
		*self.inner.original.borrow_mut() = None;
	}
}
