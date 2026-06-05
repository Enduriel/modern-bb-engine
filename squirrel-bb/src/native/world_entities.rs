//! The World entity host: `World.spawnLocation` + the native methods every world
//! entity (settlement / location / party) needs that aren't defined in script.
//!
//! BB's world entities are real `.nut` classes (`world_entity` ← `location` ←
//! `settlement`, etc.) whose *script* half defines `onInit`/`create`/gameplay, while a
//! C++ base supplies the engine primitives — identity, tile binding, and the
//! sprite/label render handles. We keep the real script classes (for mod + seed
//! fidelity) and inject the engine primitives, using the shared-method registry
//! approach of [`RootState`](crate::RootState).
//!
//! ## Why injection spans every clone level
//!
//! A BB instance is a *chain of per-level shallow clones* (`leaf` ─delegate▶ `parent`
//! ─delegate▶ `base`) that all share one member table `m` (see [`crate::class`]). A
//! super-call like `settlement.onInit` → `this.location.onInit()` rebinds `this` to the
//! per-instance *location clone* — a different object from the leaf — and that method
//! then calls injected natives (`this.setRenderedTop(…)`, `this.getTile()`). So we
//! inject the native methods onto **every clone level** of the instance (the ones
//! sharing its `m`) and register **each level's identity** against the same record;
//! whichever clone is `this` at the call site, [`SQVM::this_identity`] resolves it.
//!
//! Rendering is deliberately stubbed: `addSprite`/`addLabel` return live-but-inert
//! [`SpriteData`]/[`LabelData`] objects (readable/writable props, zero rendering).
//! `getTile` delegates to the existing `World.getTile` grid native.
//!
//! Host-owned (holds [`SqStrongRef`]s): **[`clear`](WorldEntities::clear) before VM
//! close** ([`Engine`](crate::Engine) does it).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use squirrel::{ClassBuilder, Retained, SQVM, SqClass, SqStrongRef, SqTable, Variadic};

use super::overload::{make_dispatcher, overload_key};
use crate::module::Instantiator;

/// A render sprite handle (`entity.addSprite(name)`). Inert: fields are read/written by
/// script (`Visible`, `Scale`, `Alpha`, …) but drive no rendering yet.
#[derive(Clone)]
struct SpriteData {
	#[allow(dead_code)]
	brush: String,
	alpha: f64,
	scale: f64,
	visible: bool,
	ignore_ambient: bool,
	fading_done: bool,
}

impl SpriteData {
	fn fresh() -> Self {
		SpriteData {
			brush: String::new(),
			alpha: 255.0,
			scale: 1.0,
			visible: true,
			ignore_ambient: false,
			fading_done: true, // no fade in progress
		}
	}
}

/// A text label handle (`entity.addLabel(name)`). Inert, like [`SpriteData`].
#[derive(Clone, Default)]
struct LabelData {
	text: String,
	visible: bool,
}

/// One registered world entity: its native id, the instance, its tile coords (axial),
/// and its named sprite/label handles. Reached from any of its clone levels' identities
/// via [`EntityMachine::by_identity`].
struct EntityRecord {
	id: i64,
	#[allow(dead_code)]
	entity: SqStrongRef,
	q: i64,
	r: i64,
	faction: i64,
	sprites: HashMap<String, SqStrongRef>,
	labels: HashMap<String, SqStrongRef>,
}

/// Shared entity host state: the registry, the `this.new` capability, the stub classes,
/// the id counter, and the shared native method closures to inject onto each instance.
struct EntityMachine {
	instantiate: Instantiator,
	sprite_class: Option<SqClass<SpriteData>>,
	label_class: Option<SqClass<LabelData>>,
	next_id: i64,
	entities: Vec<EntityRecord>,
	/// Every clone-level identity of every entity → its record index in `entities`.
	by_identity: HashMap<i64, usize>,
	methods: Vec<(&'static str, SqStrongRef)>,
}

impl EntityMachine {
	fn record(&self, identity: i64) -> Option<&EntityRecord> {
		self.by_identity.get(&identity).map(|&i| &self.entities[i])
	}
	fn record_mut(&mut self, identity: i64) -> Option<&mut EntityRecord> {
		let i = *self.by_identity.get(&identity)?;
		Some(&mut self.entities[i])
	}
}

/// Ignore a "no method" error (an optional hook the entity didn't define), but
/// propagate a real error raised inside one that does.
fn ignore_missing(result: Result<(), String>) -> Result<(), String> {
	match result {
		Err(e) if e.starts_with("no method") => Ok(()),
		other => other,
	}
}

/// Walk the instance's clone-level chain (the levels sharing its `m`), dropping the
/// shared native methods onto each and registering each level's identity against
/// `record_idx`. The chain ends where the delegate stops sharing `m` (i.e. at the
/// root table), so root is never touched.
fn inject_levels(
	machine: &Rc<RefCell<EntityMachine>>,
	vm: &SQVM,
	entity: &SqStrongRef,
	record_idx: usize,
) {
	let methods: Vec<(&'static str, SqStrongRef)> = machine
		.borrow()
		.methods
		.iter()
		.map(|(n, c)| (*n, c.clone()))
		.collect();
	let Some(leaf) = entity.as_table(vm) else {
		return;
	};
	let Some(shared_m) = leaf.get::<SqTable>("m").map(|m| m.to_strong().identity()) else {
		return;
	};
	let mut level = Some(leaf);
	while let Some(mut t) = level {
		// Only the instance's own clone levels share the same member table.
		if t.get::<SqTable>("m").map(|m| m.to_strong().identity()) != Some(shared_m) {
			break;
		}
		for (name, closure) in &methods {
			t.set(name, closure.clone());
		}
		machine
			.borrow_mut()
			.by_identity
			.insert(t.to_strong().identity(), record_idx);
		level = t.delegate();
	}
}

/// Instantiate `path` (runs `create()`), register it at axial `(q, r)`, inject the
/// native methods across its clone levels, then run `onInit` + `onAfterInit`. No
/// machine borrow is held across the script-running calls (they re-enter the host).
fn spawn(
	machine: &Rc<RefCell<EntityMachine>>,
	vm: &SQVM,
	path: &str,
	q: i64,
	r: i64,
) -> Result<SqStrongRef, String> {
	let instantiate = machine.borrow().instantiate.clone();
	let entity = instantiate(vm, path)?;
	let record_idx = {
		let mut m = machine.borrow_mut();
		let id = m.next_id;
		m.next_id += 1;
		let idx = m.entities.len();
		m.entities.push(EntityRecord {
			id,
			entity: entity.clone(),
			q,
			r,
			faction: 0, // Neutral until settlement.onBuild/addFaction assigns one
			sprites: HashMap::new(),
			labels: HashMap::new(),
		});
		idx
	};
	inject_levels(machine, vm, &entity, record_idx);
	entity.call_method(vm, "onInit", ())?;
	ignore_missing(entity.call_method(vm, "onAfterInit", ()))?;
	Ok(entity)
}

/// Build the inert sprite stub class returned by `addSprite`.
fn build_sprite_class(vm: &mut SQVM) -> SqClass<SpriteData> {
	ClassBuilder::<SpriteData>::new("WorldEntitySprite")
		.property(
			"Alpha",
			|s: &SpriteData| s.alpha,
			|s: &mut SpriteData, v: f64| s.alpha = v,
		)
		.property(
			"Scale",
			|s: &SpriteData| s.scale,
			|s: &mut SpriteData, v: f64| s.scale = v,
		)
		.property(
			"Visible",
			|s: &SpriteData| s.visible,
			|s: &mut SpriteData, v: bool| s.visible = v,
		)
		.property(
			"IgnoreAmbientColor",
			|s: &SpriteData| s.ignore_ambient,
			|s: &mut SpriteData, v: bool| s.ignore_ambient = v,
		)
		.property(
			"IsFadingDone",
			|s: &SpriteData| s.fading_done,
			|s: &mut SpriteData, v: bool| s.fading_done = v,
		)
		.method("setBrush", |s: &mut SpriteData, brush: &str| {
			s.brush = brush.to_string()
		})
		.method("fadeIn", |_s: &mut SpriteData, _a: Variadic<Retained>| {})
		.method("fadeOut", |_s: &mut SpriteData, _a: Variadic<Retained>| {})
		.method(
			"setHorizontalFlipping",
			|_s: &mut SpriteData, _a: Variadic<Retained>| {},
		)
		.build(vm)
}

/// Build the inert label stub class returned by `addLabel`.
fn build_label_class(vm: &mut SQVM) -> SqClass<LabelData> {
	ClassBuilder::<LabelData>::new("WorldEntityLabel")
		.property(
			"Text",
			|l: &LabelData| l.text.clone(),
			|l: &mut LabelData, v: String| l.text = v,
		)
		.property(
			"Visible",
			|l: &LabelData| l.visible,
			|l: &mut LabelData, v: bool| l.visible = v,
		)
		.build(vm)
}

/// The native World-entity subsystem (`World.spawnLocation` + injected entity methods).
pub struct WorldEntities {
	machine: Rc<RefCell<EntityMachine>>,
}

impl WorldEntities {
	/// Create the subsystem. `instantiate` is the `this.new` capability (from
	/// [`ModuleSystem::instantiator`](crate::ModuleSystem::instantiator)).
	pub fn new(instantiate: Instantiator) -> Self {
		WorldEntities {
			machine: Rc::new(RefCell::new(EntityMachine {
				instantiate,
				sprite_class: None,
				label_class: None,
				next_id: 1,
				entities: Vec::new(),
				by_identity: HashMap::new(),
				methods: Vec::new(),
			})),
		}
	}

	/// Build the stub classes + the shared entity methods, and register
	/// `World.spawnLocation` (2-arg `coords` and 3-arg `x,y` overloads). Call once with
	/// `&mut SQVM`, after the `World` table + grid natives exist.
	pub fn install(&self, vm: &mut SQVM) {
		self.machine.borrow_mut().sprite_class = Some(build_sprite_class(vm));
		self.machine.borrow_mut().label_class = Some(build_label_class(vm));

		// --- injected entity methods (each recovers its record via this_identity) ---
		let get_id = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM| -> i64 {
				m.borrow().record(vm.this_identity()).map_or(0, |r| r.id)
			})
		};
		// getTile delegates to the existing World.getTile grid native (axial coords).
		let get_tile = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM| -> Result<Option<SqStrongRef>, String> {
				let (q, r) = m
					.borrow()
					.record(vm.this_identity())
					.map(|rec| (rec.q, rec.r))
					.ok_or("getTile: entity not registered")?;
				let world = SqTable::root(vm)
					.get::<SqStrongRef>("World")
					.ok_or("getTile: World table missing")?;
				world.call_method_ret::<SqStrongRef, _>(vm, "getTile", (q, r))
			})
		};
		// getPos returns a placeholder world position (real pixel layout deferred —
		// worldgen logic uses tile coords, not pixels).
		let get_pos = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM| -> Result<Option<SqStrongRef>, String> {
				let (q, r) = m
					.borrow()
					.record(vm.this_identity())
					.map(|rec| (rec.q, rec.r))
					.ok_or("getPos: entity not registered")?;
				let create_vec = SqTable::root(vm)
					.get::<SqStrongRef>("createVec")
					.ok_or("getPos: createVec missing")?;
				create_vec.call_ret::<SqStrongRef, _>(vm, (q as f64, r as f64))
			})
		};
		let add_sprite = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM, name: String| -> Option<SqStrongRef> {
				let class = m.borrow().sprite_class.clone()?;
				let sprite = class.instantiate(vm, SpriteData::fresh());
				if let Some(rec) = m.borrow_mut().record_mut(vm.this_identity()) {
					rec.sprites.insert(name, sprite.clone());
				}
				Some(sprite)
			})
		};
		let get_sprite = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM, name: String| -> Option<SqStrongRef> {
				m.borrow()
					.record(vm.this_identity())
					.and_then(|rec| rec.sprites.get(&name).cloned())
			})
		};
		let has_sprite = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM, name: String| -> bool {
				m.borrow()
					.record(vm.this_identity())
					.is_some_and(|rec| rec.sprites.contains_key(&name))
			})
		};
		let add_label = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM, name: String| -> Option<SqStrongRef> {
				let class = m.borrow().label_class.clone()?;
				let label = class.instantiate(vm, LabelData::default());
				if let Some(rec) = m.borrow_mut().record_mut(vm.this_identity()) {
					rec.labels.insert(name, label.clone());
				}
				Some(label)
			})
		};
		let get_label = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM, name: String| -> Option<SqStrongRef> {
				m.borrow()
					.record(vm.this_identity())
					.and_then(|rec| rec.labels.get(&name).cloned())
			})
		};
		let has_label = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM, name: String| -> bool {
				m.borrow()
					.record(vm.this_identity())
					.is_some_and(|rec| rec.labels.contains_key(&name))
			})
		};
		// Faction: getFaction is native; script setFaction calls native setFactionEx
		// then updatePlayerRelation. 0 == Neutral until a faction is assigned.
		let get_faction = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM| -> i64 {
				m.borrow().record(vm.this_identity()).map_or(0, |r| r.faction)
			})
		};
		let set_faction_ex = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM, faction: i64| {
				if let Some(rec) = m.borrow_mut().record_mut(vm.this_identity()) {
					rec.faction = faction;
				}
			})
		};
		// Render/sim no-ops: accept any args, do nothing (shared across all the names).
		let noop = vm.make_closure(|_a: Variadic<Retained>| {});

		let mut methods: Vec<(&'static str, SqStrongRef)> = vec![
			("getID", get_id),
			("getTile", get_tile),
			("getPos", get_pos),
			("addSprite", add_sprite),
			("getSprite", get_sprite),
			("hasSprite", has_sprite),
			("addLabel", add_label),
			("getLabel", get_label),
			("hasLabel", has_label),
			("getFaction", get_faction),
			("setFactionEx", set_faction_ex),
		];
		for name in [
			"setPos",
			"setSpriteScaling",
			"setSpriteOffset",
			"setSpriteColorization",
			"setVisibleInFogOfWar",
			"setRenderedTop",
			"setVisibility",
			"setDiscovered",
			"registerThinker",
			"unregisterThinker",
		] {
			methods.push((name, noop.clone()));
		}
		self.machine.borrow_mut().methods = methods;

		// --- World.spawnLocation: a Sqrat overload (1 ctx / 2 coords / 3 x,y). We serve
		// the 2- and 3-arg forms; the 1-arg context form errors (worldgen never uses it).
		let spawn_2 = {
			let m = self.machine.clone();
			vm.make_closure(
				move |vm: &SQVM, path: &str, coords: SqTable| -> Result<SqStrongRef, String> {
					let q = coords
						.get::<i64>("X")
						.ok_or("spawnLocation: coords table needs X")?;
					let r = coords
						.get::<i64>("Y")
						.ok_or("spawnLocation: coords table needs Y")?;
					spawn(&m, vm, path, q, r)
				},
			)
		};
		let spawn_3 = {
			let m = self.machine.clone();
			vm.make_closure(
				move |vm: &SQVM, path: &str, x: i64, y: i64| -> Result<SqStrongRef, String> {
					spawn(&m, vm, path, x, y)
				},
			)
		};
		let dispatch = make_dispatcher(vm, "spawnLocation");

		let mut world = SqTable::root(vm)
			.get::<SqTable>("World")
			.expect("World table must be installed before WorldEntities");
		world.set(&overload_key("spawnLocation", 2), spawn_2);
		world.set(&overload_key("spawnLocation", 3), spawn_3);
		world.set("spawnLocation", dispatch);
	}

	/// Number of spawned entities (host/test inspection).
	pub fn count(&self) -> usize {
		self.machine.borrow().entities.len()
	}

	/// Release every retained ref (instances, their sprites/labels, the stub classes,
	/// the method closures). Call before the VM is closed.
	pub fn clear(&self) {
		let mut m = self.machine.borrow_mut();
		m.entities.clear();
		m.by_identity.clear();
		m.methods.clear();
		m.sprite_class = None;
		m.label_class = None;
	}
}
