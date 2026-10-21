//! Saving and loading a campaign: `World.save`, `World.load`, the four save callbacks
//! the world state registers, and the `PersistenceManager` the save/load menus use.
//! Campaigns save in the original's format ([`original`]), followed by this engine's
//! [`extension`] when it has anything to add; saves of this engine's earlier format
//! still load.
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
mod extension;
mod meta;
pub(crate) use meta::format_float as meta_format_float;
pub mod original;
mod persistence;
mod storage;
pub(crate) mod stream;

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;

use squirrel::{SQVM, SqStrongRef, SqTable, This};

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

/// One named section of the engine's extension of the original's save (see
/// [`extension`]): state beyond the original's, such as a native mod's.
pub trait ExtensionSection {
	/// The section's name, unique among sections (e.g. `"engine.factions"`, a mod's id).
	fn name(&self) -> &str;
	/// The section's bytes for a save, or `None` to leave it out.
	fn save(&self, vm: &SQVM) -> Result<Option<Vec<u8>>, String>;
	/// After a campaign loaded: the section's bytes, or `None` when the save has none (it
	/// was written without it, or the original saved the campaign again and dropped the
	/// extension).
	fn load(&self, vm: &SQVM, bytes: Option<&[u8]>) -> Result<(), String>;
}

/// The callbacks `World.setOn*Callback` registers.
#[derive(Default)]
struct Callbacks {
	before_save: Option<SaveCallback>,
	save: Option<SaveCallback>,
	before_load: Option<SaveCallback>,
	load: Option<SaveCallback>,
}

/// A registered save or load callback and the receiver it was registered on (`World`),
/// its `this` (probed: Sqrat keeps a function argument's receiver as its environment).
#[derive(Clone)]
pub(crate) struct SaveCallback {
	function: SqStrongRef,
	this: SqStrongRef,
}

impl SaveCallback {
	/// Call it with the stream. A throw is reported (the VM's error handler) and goes no
	/// further: the original saves or loads on (probed 2026-10-21).
	pub(crate) fn run(&self, vm: &SQVM, stream: &SqStrongRef) {
		drop(self.function.call_with_this(vm, &self.this, (stream.clone(),)));
	}
}

struct Inner {
	storage: RefCell<Storage>,
	parts: RefCell<Vec<Rc<dyn SavePart>>>,
	/// The extension's sections, by name.
	sections: RefCell<BTreeMap<String, Rc<dyn ExtensionSection>>>,
	/// The last loaded save's sections that no [`ExtensionSection`] here owns: written
	/// back as they were.
	kept: RefCell<BTreeMap<String, Vec<u8>>>,
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
	/// This engine's own earlier format.
	Own,
	/// The original's: its container and its layout, which the original game loads,
	/// followed by this engine's extension when there's anything to put in it.
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
fn run_callback(vm: &SQVM, callback: Option<SaveCallback>, stream: &SqStrongRef) {
	if let Some(c) = callback {
		c.run(vm, stream);
	}
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
	let original = inner.format.get() == SaveFormat::Original;
	// A world map past the original's clamp saves in the same layout, marked so the
	// original refuses it (see [`original::OVER_LIMIT`]).
	let over_limit = inner.original.borrow().as_ref().is_some_and(|world| {
		let (w, h) = world.map_size();
		w > original::MAX_MAP.0 || h > original::MAX_MAP.1
	});
	let writer = Rc::new(RefCell::new(SaveWriter {
		strings: if original { stream::StrLen::U16 } else { stream::StrLen::U32 },
		..SaveWriter::default()
	}));
	let stream = classes.out_stream(vm, writer.clone(), meta.clone());
	let before = inner.callbacks.borrow().before_save.clone();
	run_callback(vm, before, &stream);
	if original {
		let world = inner.original.borrow().clone().ok_or("saving in the original's format: no world to save")?;
		let after = inner.callbacks.borrow().save.clone();
		let cx = SaveCx { vm, writer: writer.clone(), stream: stream.clone() };
		original::export::save(world.as_ref(), &cx, after)?;
		if over_limit {
			meta.borrow_mut().version |= original::OVER_LIMIT;
		}
		let sections = extension_sections(inner, vm)?;
		if !sections.is_empty() {
			writer.borrow_mut().buf.extend_from_slice(&extension::encode(&sections));
		}
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
	run_callback(vm, after, &stream);
	let bytes = storage::encode(&meta.borrow(), &writer.borrow().buf);
	inner.storage.borrow().write(&file, bytes)
}

/// Log `message` as the scripts' `logWarning` does.
fn warn(vm: &SQVM, message: &str) {
	if let Some(log) = SqTable::root(vm).get::<SqStrongRef>("logWarning") {
		let _ = log.call(vm, (message.to_string(),));
	}
}

/// The extension's sections for a save: each owner's, and those kept from the last load
/// that nothing here owns.
fn extension_sections(inner: &Inner, vm: &SQVM) -> Result<BTreeMap<String, Vec<u8>>, String> {
	let mut out = inner.kept.borrow().clone();
	let sections: Vec<_> = inner.sections.borrow().values().cloned().collect();
	for section in sections {
		out.remove(section.name());
		if let Some(bytes) = section.save(vm)? {
			out.insert(section.name().to_string(), bytes);
		}
	}
	Ok(out)
}

/// Hand each owner its section (or `None`), and keep the sections nothing owns.
fn load_extension(inner: &Inner, vm: &SQVM, mut found: BTreeMap<String, Vec<u8>>) -> Result<(), String> {
	let sections: Vec<_> = inner.sections.borrow().values().cloned().collect();
	for section in &sections {
		section.load(vm, found.get(section.name()).map(Vec::as_slice))?;
		found.remove(section.name());
	}
	*inner.kept.borrow_mut() = found;
	Ok(())
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
		original::import::load(world.as_ref(), &cx, version, before, after)?;
		let found = extension::decode(cx.reader.borrow().rest()).unwrap_or_else(|e| {
			// The original loads the save regardless; so does this engine.
			warn(vm, &format!("Loading \"{file}\": {e}; the extension is ignored"));
			BTreeMap::new()
		});
		return load_extension(inner, vm, found);
	}
	let before = inner.callbacks.borrow().before_load.clone();
	run_callback(vm, before, &stream);
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
	run_callback(vm, after, &stream);
	Ok(())
}

impl SaveSystem {
	/// A save system keeping saves in memory until [`set_directory`](Self::set_directory).
	pub fn new() -> Self {
		SaveSystem {
			inner: Rc::new(Inner {
				storage: RefCell::new(Storage::Memory(RefCell::default())),
				parts: RefCell::default(),
				sections: RefCell::default(),
				kept: RefCell::default(),
				original: RefCell::default(),
				format: Cell::new(SaveFormat::Original),
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

	/// Save campaigns in `format` (the original's unless set). Saves of either format
	/// load.
	pub fn set_format(&self, format: SaveFormat) {
		self.inner.format.set(format);
	}

	/// The engine's parts for loading (and writing) the original's layout.
	pub(crate) fn set_original_world(&self, world: Rc<dyn original::world::OriginalWorld>) {
		*self.inner.original.borrow_mut() = Some(world);
	}

	/// Add a section to the extension of the original's save. A section of the same name
	/// replaces the earlier one.
	pub fn add_extension_section(&self, section: Rc<dyn ExtensionSection>) {
		self.inner.sections.borrow_mut().insert(section.name().to_string(), section);
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
				vm.make_closure(move |vm: &SQVM, this: This, callback: Option<SqStrongRef>| {
					let this = this.to_strong(vm);
					inner.callbacks.borrow_mut().$field = callback.map(|function| SaveCallback { function, this });
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
		self.inner.sections.borrow_mut().clear();
		*self.inner.original.borrow_mut() = None;
	}
}
