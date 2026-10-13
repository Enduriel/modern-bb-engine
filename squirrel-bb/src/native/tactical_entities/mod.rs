//! The tactical-entity / roster host: `World.getRoster`/`getPlayerRoster`/`createRoster`
//! return real rosters, and `roster.create(scriptPath)` instantiates a real combat unit
//! (`entity → actor → human → councilman`/`player`, etc.), injecting the engine
//! primitives those classes expect.
//!
//! This mirrors [`WorldEntities`](crate::native::WorldEntities): a `this_identity`
//! registry, natives injected on the instance's base class level and bound to the leaf
//! (see [`super::inject`]), inert sprite stubs, and the module [`Instantiator`] for
//! script-class loading. It differs in
//! lifecycle: tactical entities live in rosters (not at world `(q,r)` tiles), start with
//! no faction (0, whatever roster creates them; scripts then call `setFaction`, as
//! probed in the original 2026-10-13), and they are not placed on the world map
//! (`getTile` → null, `isPlacedOnMap` → false).
//!
//! Skills/items/backgrounds run as REAL script classes (mod + seed fidelity); only the
//! visual surface (sprites/overlay) is inert. Host-owned: **[`clear`](TacticalEntities::clear)
//! before VM close** (holds the rosters' + entities' [`SqStrongRef`]s).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use squirrel::{ClassBuilder, Retained, SQVM, SqArray, SqClass, SqStrongRef, SqTable, Variadic};

use super::inject::inject_natives;

mod persist;
use crate::module::Instantiator;

/// The player company roster's id (real faction/settlement roster ids are >= 0).
const PLAYER_ROSTER_ID: i64 = -1;

/// An inert render sprite handle for a tactical unit. Richer than the world sprite stub
/// (units read more back): also `Rotation`, `Color` (no-op), `HasBrush`, `getBrush`,
/// `varyColor`/`varySaturation`/`setHorizontalFlipping` — all visual no-ops headless.
#[derive(Clone)]
struct UnitSpriteData {
	brush: String,
	alpha: f64,
	scale: f64,
	rotation: f64,
	saturation: f64,
	render_anim_start: f64,
	visible: bool,
	raising_weapon: bool,
	lowering_weapon: bool,
	raising_shield: bool,
	lowering_shield: bool,
}

impl UnitSpriteData {
	fn fresh() -> Self {
		UnitSpriteData {
			brush: String::new(),
			alpha: 255.0,
			scale: 1.0,
			rotation: 0.0,
			saturation: 1.0,
			render_anim_start: 0.0,
			visible: true,
			raising_weapon: false,
			lowering_weapon: false,
			raising_shield: false,
			lowering_shield: false,
		}
	}
}

/// The payload behind a roster handle: just its id (the roster's contents live in the
/// machine, keyed by this id, so any handle to the same id shares state).
#[derive(Clone)]
struct RosterData {
	id: i64,
}

/// One registered tactical unit: its native id, the instance, and its faction (0 until a
/// script calls `setFaction`).
struct UnitRecord {
	id: i64,
	/// The script it was created from (to re-create it on load).
	path: String,
	entity: SqStrongRef,
	faction: i64,
	/// `isInitialized()`: false during the first `onInit` (so actor.onInit's one-time
	/// property/overlay setup runs), then set true once spawn finishes onInit.
	initialized: bool,
	/// `isDiscovered`/`setDiscovered` (false until player.onInit sets it, like the original).
	discovered: bool,
	/// `isBlockingSight`/`setBlockSight`.
	blocks_sight: bool,
	/// `getPos`/`setPos` world position (0,0 while off any map, like the original).
	pos: (f64, f64),
	sprites: HashMap<String, SqStrongRef>,
}

/// Shared host state: the rosters, the unit registry, the `this.new` capability, the stub
/// classes, the id counter, and the shared injected method closures.
struct TacticalMachine {
	instantiate: Instantiator,
	rosters: HashMap<i64, Vec<SqStrongRef>>,
	units: Vec<UnitRecord>,
	by_identity: HashMap<i64, usize>,
	roster_class: Option<SqClass<RosterData>>,
	sprite_class: Option<SqClass<UnitSpriteData>>,
	next_id: i64,
	methods: Vec<(&'static str, SqStrongRef)>,
}

impl TacticalMachine {
	fn record(&self, identity: i64) -> Option<&UnitRecord> {
		self.by_identity.get(&identity).map(|&i| &self.units[i])
	}
	fn record_mut(&mut self, identity: i64) -> Option<&mut UnitRecord> {
		let i = *self.by_identity.get(&identity)?;
		Some(&mut self.units[i])
	}
}

/// Ignore a "no method" error (optional hook the class didn't define); propagate real ones.
fn ignore_missing(result: Result<(), String>) -> Result<(), String> {
	match result {
		Err(e) if e.starts_with("no method") => Ok(()),
		other => other,
	}
}

/// Inject the shared natives into `entity` (on its base class level, bound to the
/// leaf; see [`inject_natives`]) and register every level's identity against
/// `record_idx`.
fn inject_levels(
	machine: &Rc<RefCell<TacticalMachine>>,
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
	for identity in inject_natives(vm, &leaf, &methods) {
		machine.borrow_mut().by_identity.insert(identity, record_idx);
	}
}

/// Instantiate `path` (runs `create()`) as unit `id` with no faction, and inject the
/// natives (base class level, bound to the leaf). Returns the instance and its record
/// index. Runs no other script.
fn create_unit(
	machine: &Rc<RefCell<TacticalMachine>>,
	vm: &SQVM,
	path: &str,
	id: i64,
) -> Result<(SqStrongRef, usize), String> {
	let instantiate = machine.borrow().instantiate.clone();
	let entity = instantiate(vm, path)?;
	let record_idx = {
		let mut m = machine.borrow_mut();
		let idx = m.units.len();
		m.units.push(UnitRecord {
			id,
			path: path.to_string(),
			entity: entity.clone(),
			faction: 0,
			initialized: false,
			discovered: false,
			blocks_sight: false,
			pos: (0.0, 0.0),
			sprites: HashMap::new(),
		});
		idx
	};
	inject_levels(machine, vm, &entity, record_idx);
	Ok((entity, record_idx))
}

/// Create a new unit of `path` with the next id, then run `onInit`/`onAfterInit`.
fn spawn(machine: &Rc<RefCell<TacticalMachine>>, vm: &SQVM, path: &str) -> Result<SqStrongRef, String> {
	let id = {
		let mut m = machine.borrow_mut();
		m.next_id += 1;
		m.next_id - 1
	};
	let (entity, record_idx) = create_unit(machine, vm, path, id)?;
	entity.call_method(vm, "onInit", ())?;
	ignore_missing(entity.call_method(vm, "onAfterInit", ()))?;
	// The one-time onInit setup has run; the entity is now initialized.
	machine.borrow_mut().units[record_idx].initialized = true;
	Ok(entity)
}

/// Build the inert unit sprite stub class.
fn build_sprite_class(vm: &mut SQVM) -> SqClass<UnitSpriteData> {
	ClassBuilder::<UnitSpriteData>::new("TacticalUnitSprite")
		.property(
			"Alpha",
			|s: &UnitSpriteData| s.alpha,
			|s: &mut UnitSpriteData, v: f64| s.alpha = v,
		)
		.property(
			"Scale",
			|s: &UnitSpriteData| s.scale,
			|s: &mut UnitSpriteData, v: f64| s.scale = v,
		)
		.property(
			"Rotation",
			|s: &UnitSpriteData| s.rotation,
			|s: &mut UnitSpriteData, v: f64| s.rotation = v,
		)
		.property(
			"Visible",
			|s: &UnitSpriteData| s.visible,
			|s: &mut UnitSpriteData, v: bool| s.visible = v,
		)
		.property(
			"Saturation",
			|s: &UnitSpriteData| s.saturation,
			|s: &mut UnitSpriteData, v: f64| s.saturation = v,
		)
		.property(
			"RenderAnimationStartTime",
			|s: &UnitSpriteData| s.render_anim_start,
			|s: &mut UnitSpriteData, v: f64| s.render_anim_start = v,
		)
		.property(
			"IsRaisingWeapon",
			|s: &UnitSpriteData| s.raising_weapon,
			|s: &mut UnitSpriteData, v: bool| s.raising_weapon = v,
		)
		.property(
			"IsLoweringWeapon",
			|s: &UnitSpriteData| s.lowering_weapon,
			|s: &mut UnitSpriteData, v: bool| s.lowering_weapon = v,
		)
		.property(
			"IsRaisingShield",
			|s: &UnitSpriteData| s.raising_shield,
			|s: &mut UnitSpriteData, v: bool| s.raising_shield = v,
		)
		.property(
			"IsLoweringShield",
			|s: &UnitSpriteData| s.lowering_shield,
			|s: &mut UnitSpriteData, v: bool| s.lowering_shield = v,
		)
		.property_noop("Color")
		.method("setBrush", |s: &mut UnitSpriteData, brush: &str| {
			s.brush = brush.to_string()
		})
		// getBrush(): a minimal brush stub carrying just `.Name` (the stored brush id),
		// which is what setAppearance reads (`getBrush().Name + "_top"`).
		.method("getBrush", |s: &mut UnitSpriteData, vm: &SQVM| -> SqStrongRef {
			let mut t = SqTable::new(vm);
			t.set("Name", s.brush.clone());
			t.to_strong()
		})
		.method("resetBrush", |s: &mut UnitSpriteData| s.brush = String::new())
		.method(
			"setRenderCallbackEnabled",
			|_s: &mut UnitSpriteData, _a: Variadic<Retained>| {},
		)
		.method("setHorizontalFlipping", |_s: &mut UnitSpriteData, _f: bool| {})
		.method("varyColor", |_s: &mut UnitSpriteData, _a: Variadic<Retained>| {})
		.method(
			"varySaturation",
			|_s: &mut UnitSpriteData, _a: Variadic<Retained>| {},
		)
		.method(
			"varyBrightness",
			|_s: &mut UnitSpriteData, _a: Variadic<Retained>| {},
		)
		.method(
			"setBrightness",
			|_s: &mut UnitSpriteData, _a: Variadic<Retained>| {},
		)
		.build(vm)
}

/// Build the roster handle class (`getSize`/`getAll`/`create`/`add`/`remove`/`clear`).
fn build_roster_class(
	vm: &mut SQVM,
	machine: &Rc<RefCell<TacticalMachine>>,
) -> SqClass<RosterData> {
	let m_size = machine.clone();
	let m_all = machine.clone();
	let m_create = machine.clone();
	let m_add = machine.clone();
	let m_remove = machine.clone();
	let m_clear = machine.clone();
	ClassBuilder::<RosterData>::new("TacticalRoster")
		.method("getSize", move |r: &mut RosterData| -> i64 {
			m_size
				.borrow()
				.rosters
				.get(&r.id)
				.map_or(0, |v| v.len() as i64)
		})
		.method("getAll", move |r: &mut RosterData, vm: &SQVM| -> SqStrongRef {
			let mut arr = SqArray::new(vm);
			if let Some(v) = m_all.borrow().rosters.get(&r.id) {
				for e in v {
					arr.push(e.clone());
				}
			}
			arr.to_strong()
		})
		.method(
			"create",
			move |r: &mut RosterData, vm: &SQVM, path: &str| -> Result<SqStrongRef, String> {
				let entity = spawn(&m_create, vm, path)?;
				m_create
					.borrow_mut()
					.rosters
					.entry(r.id)
					.or_default()
					.push(entity.clone());
				Ok(entity)
			},
		)
		.method("add", move |r: &mut RosterData, entity: Retained| {
			m_add
				.borrow_mut()
				.rosters
				.entry(r.id)
				.or_default()
				.push(entity.0);
		})
		.method("remove", move |r: &mut RosterData, entity: Retained| {
			let id = entity.0.identity();
			if let Some(v) = m_remove.borrow_mut().rosters.get_mut(&r.id) {
				v.retain(|e| e.identity() != id);
			}
		})
		.method("clear", move |r: &mut RosterData| {
			if let Some(v) = m_clear.borrow_mut().rosters.get_mut(&r.id) {
				v.clear();
			}
		})
		.build(vm)
}

/// The native tactical-entity / roster subsystem.
pub struct TacticalEntities {
	machine: Rc<RefCell<TacticalMachine>>,
}

impl TacticalEntities {
	/// Create the subsystem. `instantiate` is `this.new` (from
	/// [`ModuleSystem::instantiator`](crate::ModuleSystem::instantiator)).
	pub fn new(instantiate: Instantiator) -> Self {
		TacticalEntities {
			machine: Rc::new(RefCell::new(TacticalMachine {
				instantiate,
				rosters: HashMap::new(),
				units: Vec::new(),
				by_identity: HashMap::new(),
				roster_class: None,
				sprite_class: None,
				next_id: 1,
				methods: Vec::new(),
			})),
		}
	}

	/// Build the roster + sprite classes and the injected unit methods, then wire
	/// `World.getRoster`/`getPlayerRoster`/`createRoster`/`deleteRoster`. Call once with
	/// `&mut SQVM`, after the `World` table exists.
	pub fn install(&self, vm: &mut SQVM) {
		self.machine.borrow_mut().sprite_class = Some(build_sprite_class(vm));

		// --- injected unit methods (recover the record via this_identity) ---
		let get_id = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM| -> i64 {
				m.borrow().record(vm.this_identity()).map_or(0, |r| r.id)
			})
		};
		let get_faction = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM| -> i64 {
				m.borrow().record(vm.this_identity()).map_or(0, |r| r.faction)
			})
		};
		// setFactionEx(f): the native half of actor.setFaction (which handles the
		// map-side bookkeeping in script).
		let set_faction_ex = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM, faction: i64| {
				if let Some(rec) = m.borrow_mut().record_mut(vm.this_identity()) {
					rec.faction = faction;
				}
			})
		};
		let add_sprite = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM, name: String| -> Option<SqStrongRef> {
				let class = m.borrow().sprite_class.clone()?;
				let sprite = class.instantiate(vm, UnitSpriteData::fresh());
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
		// createOverlay returns an (inert) sprite too; updateOverlay/etc. are no-ops.
		let create_overlay = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM, _a: Variadic<Retained>| -> Option<SqStrongRef> {
				let class = m.borrow().sprite_class.clone()?;
				Some(class.instantiate(vm, UnitSpriteData::fresh()))
			})
		};
		let is_initialized = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM| -> bool {
				m.borrow().record(vm.this_identity()).is_some_and(|r| r.initialized)
			})
		};
		let is_placed_on_map = vm.make_closure(|| false);
		let get_tile = vm.make_closure(|| None::<SqStrongRef>); // not on the world map
		// getSpriteOffset(name): sprite offsets are visual; return a zero vec (the one
		// caller, actor.onAfterInit, only feeds it straight into no-op setSpriteOffset).
		let get_sprite_offset = vm.make_closure(
			move |vm: &SQVM, _name: Variadic<Retained>| -> Result<Option<SqStrongRef>, String> {
				let create_vec = SqTable::root(vm)
					.get::<SqStrongRef>("createVec")
					.ok_or("getSpriteOffset: createVec missing")?;
				create_vec.call_ret::<SqStrongRef, _>(vm, (0.0_f64, 0.0_f64))
			},
		);
		let noop = vm.make_closure(|_a: Variadic<Retained>| {});
		// Unit state the scripts read back (semantics probed in the original, 2026-10-13).
		let flag = |vm: &mut SQVM, m: &Rc<RefCell<TacticalMachine>>, get: fn(&UnitRecord) -> bool, set: fn(&mut UnitRecord, bool)| {
			let mg = m.clone();
			let getter = vm.make_closure(move |vm: &SQVM| -> bool { mg.borrow().record(vm.this_identity()).is_some_and(get) });
			let ms = m.clone();
			let setter = vm.make_closure(move |vm: &SQVM, v: bool| {
				if let Some(rec) = ms.borrow_mut().record_mut(vm.this_identity()) {
					set(rec, v);
				}
			});
			(getter, setter)
		};
		let (is_discovered, set_discovered) =
			flag(vm, &self.machine, |r| r.discovered, |r, v| r.discovered = v);
		let (is_blocking_sight, set_block_sight) =
			flag(vm, &self.machine, |r| r.blocks_sight, |r, v| r.blocks_sight = v);
		// Not on a tactical map headless, so never hidden (the original says false even
		// for an undiscovered unit off the map).
		let is_hidden_to_player = vm.make_closure(|| false);
		let get_pos = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM| -> Result<Option<SqStrongRef>, String> {
				let (x, y) = m.borrow().record(vm.this_identity()).map_or((0.0, 0.0), |r| r.pos);
				let create_vec = SqTable::root(vm)
					.get::<SqStrongRef>("createVec")
					.ok_or("getPos: createVec missing")?;
				create_vec.call_ret::<SqStrongRef, _>(vm, (x, y))
			})
		};
		let set_pos = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM, pos: Retained| {
				let x = pos.0.get_member::<squirrel::Number>(vm, "X").map_or(0.0, |n| n.0);
				let y = pos.0.get_member::<squirrel::Number>(vm, "Y").map_or(0.0, |n| n.0);
				if let Some(rec) = m.borrow_mut().record_mut(vm.this_identity()) {
					rec.pos = (x, y);
				}
			})
		};

		let mut methods: Vec<(&'static str, SqStrongRef)> = vec![
			("getID", get_id),
			("getFaction", get_faction),
			("setFactionEx", set_faction_ex),
			("addSprite", add_sprite),
			("getSprite", get_sprite),
			("hasSprite", has_sprite),
			("getSpriteOffset", get_sprite_offset),
			("createOverlay", create_overlay),
			("isInitialized", is_initialized),
			("isPlacedOnMap", is_placed_on_map),
			("getTile", get_tile),
			("isDiscovered", is_discovered),
			("setDiscovered", set_discovered),
			("isBlockingSight", is_blocking_sight),
			("setBlockSight", set_block_sight),
			("isHiddenToPlayer", is_hidden_to_player),
			("getPos", get_pos),
			("setPos", set_pos),
		];
		for name in [
			"setDirty",
			"setVisibility",
			"setVisibleInFogOfWar",
			"setPreventOcclusion",
			"setSpriteScaling",
			"setSpriteOffset",
			"setSpriteColorization",
			"addDefaultStatusSprites",
			"updateOverlay",
			"setOverlayValues",
			"setOverlayIcons",
			// Render-only, or map-only while units aren't placed:
			"fadeIn",
			"fadeOut",
			"setAlpha",
			"moveSpriteOffset",
			"setRenderCallbackEnabled",
			"updateVisibility",
			"removeFromMap",
		] {
			methods.push((name, noop.clone()));
		}
		self.machine.borrow_mut().methods = methods;

		// Roster handle class + the World roster accessors.
		let roster_class = build_roster_class(vm, &self.machine);
		self.machine.borrow_mut().roster_class = Some(roster_class);

		let mint_roster = {
			let m = self.machine.clone();
			move |vm: &SQVM, id: i64| -> Option<SqStrongRef> {
				let class = m.borrow().roster_class.clone()?;
				m.borrow_mut().rosters.entry(id).or_default();
				Some(class.instantiate(vm, RosterData { id }))
			}
		};
		let get_roster = {
			let mint = mint_roster.clone();
			vm.make_closure(move |vm: &SQVM, id: i64| -> Option<SqStrongRef> { mint(vm, id) })
		};
		let get_player_roster = {
			let mint = mint_roster.clone();
			vm.make_closure(move |vm: &SQVM| -> Option<SqStrongRef> { mint(vm, PLAYER_ROSTER_ID) })
		};
		let create_roster = {
			let m = self.machine.clone();
			vm.make_closure(move |id: i64| {
				m.borrow_mut().rosters.entry(id).or_default();
			})
		};
		let delete_roster = {
			let m = self.machine.clone();
			vm.make_closure(move |id: i64| {
				m.borrow_mut().rosters.remove(&id);
			})
		};

		// Tactical.getEntityByID(id): a unit that still exists, i.e. is in a roster (a
		// unit removed from every roster is gone, so e.g. a dead employer's contract
		// becomes invalid).
		let get_entity_by_id = {
			let m = self.machine.clone();
			vm.make_closure(move |id: i64| -> Option<SqStrongRef> {
				let machine = m.borrow();
				let unit = machine.units.iter().find(|u| u.id == id)?;
				let identity = unit.entity.identity();
				machine
					.rosters
					.values()
					.flatten()
					.find(|e| e.identity() == identity)
					.cloned()
			})
		};
		let mut world = SqTable::root(vm)
			.get::<SqTable>("World")
			.expect("World table must be installed before TacticalEntities");
		world.set("getRoster", get_roster);
		world.set("getPlayerRoster", get_player_roster);
		world.set("createRoster", create_roster);
		world.set("deleteRoster", delete_roster);
		let mut tactical = SqTable::root(vm)
			.get::<SqTable>("Tactical")
			.expect("Tactical table must be installed before TacticalEntities");
		tactical.set("getEntityByID", get_entity_by_id);
	}

	/// The rosters' share of a save (see [`persist`]).
	pub(crate) fn save_part(&self) -> Rc<dyn crate::native::save::SavePart> {
		Rc::new(persist::RostersPart(self.machine.clone()))
	}

	/// Total units created (host/test inspection).
	pub fn unit_count(&self) -> usize {
		self.machine.borrow().units.len()
	}

	/// Release every retained ref (rosters, units, their sprites, the classes, methods).
	/// Call before the VM is closed.
	pub fn clear(&self) {
		let mut m = self.machine.borrow_mut();
		m.rosters.clear();
		m.units.clear();
		m.by_identity.clear();
		m.methods.clear();
		m.roster_class = None;
		m.sprite_class = None;
	}
}
