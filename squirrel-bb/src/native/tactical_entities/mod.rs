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
use super::overload::{make_dispatcher, overload_key};

mod persist;
mod effects;
mod placement;
mod render;
mod vision;
use crate::module::Instantiator;
use crate::native::Sprites;

/// The player company roster's id (real faction/settlement roster ids are >= 0).
const PLAYER_ROSTER_ID: i64 = -1;
/// The other fixed rosters: the world's guests (`World.getGuestRoster`), and a battle's
/// casualties, survivors, retreated units and temporary units (`Tactical.get…Roster`).
const GUEST_ROSTER_ID: i64 = -2;
const CASUALTY_ROSTER_ID: i64 = -3;
const SURVIVOR_ROSTER_ID: i64 = -4;
const RETREAT_ROSTER_ID: i64 = -5;
const TEMPORARY_ROSTER_ID: i64 = -6;

/// A render sprite handle for a tactical unit. Richer than the world sprite (units read
/// more back): also `Rotation`, `Color`, `HasBrush`, `getBrush` and
/// `setHorizontalFlipping`. `varyColor`/`varySaturation` and the fades are still no-ops
/// here (the fades' faithful, draw-time semantics live in `SpriteColor`, used by world
/// sprites and tactical details).
#[derive(Clone)]
struct UnitSpriteData {
	brush: String,
	/// `Color` (ARGB). Skeletons copy their body's into their blood colour.
	color: u32,
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
	/// `setHorizontalFlipping` / `isFlippedHorizontally`.
	flipped: bool,
	highlight: bool,
	highlight_factor: f64,
	finished: bool,
	fading: bool,
	fading_done: bool,
	torque: f64,
	ignore_ambient: bool,
	ignore_camera_flip: bool,
}

impl UnitSpriteData {
	fn fresh() -> Self {
		UnitSpriteData {
			brush: String::new(),
			color: 0xffff_ffff,
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
			flipped: false,
			highlight: false,
			highlight_factor: 0.0,
			finished: false,
			fading: false,
			fading_done: true,
			torque: 0.0,
			ignore_ambient: false,
			ignore_camera_flip: false,
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
	/// The battle tile it stands on, if placed.
	tile: Option<usize>,
	/// `isActor()`: a unit (its class derives from `actor`) rather than a map object.
	is_actor: bool,
	/// `die()` ran: gone from the battle (`getEntityByID` no longer finds it).
	dead: bool,
	/// `setRenderCallbackEnabled`: its `onRender` runs when it is drawn (see `render`).
	render_callback: bool,
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
	/// The battle map, for placing units and objects.
	grid: Option<crate::native::tactical_grid::Grid>,
	/// Resolves sprite brushes (set by the host before `install`).
	sprites: Sprites,
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
	let is_actor = SqTable::root(vm)
		.get::<SqStrongRef>("isKindOf")
		.and_then(|f| f.call_ret::<bool, _>(vm, (entity.clone(), "actor".to_string())).ok().flatten())
		.unwrap_or(false);
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
			tile: None,
			is_actor,
			dead: false,
			render_callback: false,
			sprites: HashMap::new(),
		});
		idx
	};
	inject_levels(machine, vm, &entity, record_idx);
	set_parent_id(vm, &entity, 0);
	Ok((entity, record_idx))
}

/// `m.ParentID`: the original adds it to the base level of every tactical entity's `m`
/// (where `entity.nut`'s own fields live), 0 unless the 4-argument `spawnEntity` gave
/// an id (the lindwurm tail's body, which its `onInit` reads). Probed 2026-10-13.
pub(super) fn set_parent_id(vm: &SQVM, entity: &SqStrongRef, parent: i64) {
	let Some(mut m) = entity.as_table(vm).and_then(|t| t.get::<SqTable>("m")) else { return };
	while let Some(d) = m.delegate() {
		m = d;
	}
	m.set("ParentID", parent);
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

/// Build the unit sprite class. Brushes resolve through `sprites` (see
/// [`Sprites::resolve`]).
fn build_sprite_class(vm: &mut SQVM, sprites: &Sprites) -> SqClass<UnitSpriteData> {
	let (set_with, get_with) = (sprites.clone(), sprites.clone());
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
		.property_obj_rw(
			"Color",
			|s: &UnitSpriteData, vm: &SQVM| super::color::color_object(vm, s.color),
			|s: &mut UnitSpriteData, vm: &SQVM, v: SqStrongRef| {
				s.color = super::color::color_argb(vm, &v).ok_or("Color: not a color")?;
				Ok(())
			},
		)
		.method("setBrush", move |s: &mut UnitSpriteData, brush: &str| s.brush = set_with.resolve(brush))
		// getBrush(): `null` without a brush, else `{ Name, TextureName }`; scripts read
		// `getBrush().Name + "_top"` and hand `TextureName` to `Tactical.addResource`.
		.method("getBrush", move |s: &mut UnitSpriteData, vm: &SQVM| -> Option<SqStrongRef> {
			get_with.brush_object(vm, &s.brush)
		})
		.method("resetBrush", |s: &mut UnitSpriteData| s.brush = String::new())
		.method(
			"setRenderCallbackEnabled",
			|_s: &mut UnitSpriteData, _a: Variadic<Retained>| {},
		)
		.method("setHorizontalFlipping", |s: &mut UnitSpriteData, f: bool| s.flipped = f)
		.method("isFlippedHorizontally", |s: &mut UnitSpriteData| s.flipped)
		.property("HasBrush", |s: &UnitSpriteData| !s.brush.is_empty(), |_s: &mut UnitSpriteData, _v: bool| {})
		.property("Highlight", |s: &UnitSpriteData| s.highlight, |s: &mut UnitSpriteData, v: bool| s.highlight = v)
		.property("HighlightFactor", |s: &UnitSpriteData| s.highlight_factor, |s: &mut UnitSpriteData, v: f64| s.highlight_factor = v)
		.property("Finished", |s: &UnitSpriteData| s.finished, |s: &mut UnitSpriteData, v: bool| s.finished = v)
		.property("Fading", |s: &UnitSpriteData| s.fading, |s: &mut UnitSpriteData, v: bool| s.fading = v)
		.property("IsFadingDone", |s: &UnitSpriteData| s.fading_done, |s: &mut UnitSpriteData, v: bool| s.fading_done = v)
		.property("Torque", |s: &UnitSpriteData| s.torque, |s: &mut UnitSpriteData, v: f64| s.torque = v)
		.property("IgnoreAmbientColor", |s: &UnitSpriteData| s.ignore_ambient, |s: &mut UnitSpriteData, v: bool| s.ignore_ambient = v)
		.property("IgnoreCameraFlip", |s: &UnitSpriteData| s.ignore_camera_flip, |s: &mut UnitSpriteData, v: bool| s.ignore_camera_flip = v)
		// Animation and fades are visual; a fade that hides does so at once headless.
		.method("fadeOutAndHide", |s: &mut UnitSpriteData, _a: Variadic<Retained>| s.visible = false)
		.method("play", |_s: &mut UnitSpriteData, _a: Variadic<Retained>| {})
		.method("stop", |_s: &mut UnitSpriteData, _a: Variadic<Retained>| {})
		.method("finish", |_s: &mut UnitSpriteData, _a: Variadic<Retained>| {})
		.method("fade", |_s: &mut UnitSpriteData, _a: Variadic<Retained>| {})
		.method("fadeIn", |_s: &mut UnitSpriteData, _a: Variadic<Retained>| {})
		.method("fadeOut", |_s: &mut UnitSpriteData, _a: Variadic<Retained>| {})
		.method("fadeToAlpha", |_s: &mut UnitSpriteData, _a: Variadic<Retained>| {})
		.method("setOffset", |_s: &mut UnitSpriteData, _a: Variadic<Retained>| {})
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
/// What other services need of a unit: its faction and the tile it stands on.
pub(crate) struct UnitInfo {
	pub faction: i64,
	pub tile: Option<usize>,
}

/// The identity a native should look a unit up by: the value's own, or, for a
/// `WeakTableRef` (how AI code holds units), the referenced table's. The original's
/// natives resolve their argument through its `CppInstance`, which a `WeakTableRef`
/// forwards to the table the same way.
pub(crate) fn unit_identity(vm: &SQVM, value: &SqStrongRef) -> i64 {
	value
		.get_member::<SqStrongRef>(vm, "WeakTable")
		.filter(|t| t.as_table(vm).is_some())
		.map_or_else(|| value.identity(), |t| t.identity())
}

/// What the navigator does to units: look them up, move them between tiles, discover
/// them. (A handle on the entity host's records; no borrow outlives a call.)
#[derive(Clone)]
pub(crate) struct Units(Rc<RefCell<TacticalMachine>>);

impl Units {
	/// Faction and tile of the unit with this identity.
	pub fn info(&self, identity: i64) -> Option<UnitInfo> {
		self.0.borrow().record(identity).map(|r| UnitInfo {
			faction: r.faction,
			tile: r.tile,
		})
	}

	/// The unit's script instance.
	pub fn entity(&self, identity: i64) -> Option<SqStrongRef> {
		self.0.borrow().record(identity).map(|r| r.entity.clone())
	}

	/// Move the unit onto `tile` the way the original's set-tile does (see
	/// [`placement::place`]); no callbacks.
	pub fn move_to(&self, identity: i64, tile: usize) -> Result<(), String> {
		let (idx, is_actor) = {
			let m = self.0.borrow();
			let idx = *m.by_identity.get(&identity).ok_or("move: not a tactical unit")?;
			(idx, m.units[idx].is_actor)
		};
		placement::place(&self.0, idx, tile, is_actor)
	}

	/// Take the unit off its tile (it stays in the battle); no callbacks.
	pub fn lift(&self, identity: i64) -> Result<(), String> {
		let idx = *self.0.borrow().by_identity.get(&identity).ok_or("lift: not a tactical unit")?;
		placement::unplace(&self.0, idx);
		Ok(())
	}

	/// Discover the unit for the player; a newly discovered one runs `onDiscovered`.
	pub fn discover(&self, vm: &SQVM, identity: i64) -> Result<(), String> {
		let newly = {
			let mut m = self.0.borrow_mut();
			m.record_mut(identity).and_then(|r| {
				let newly = !r.discovered;
				r.discovered = true;
				newly.then(|| r.entity.clone())
			})
		};
		match newly {
			Some(e) => ignore_missing(e.call_method(vm, "onDiscovered", ())),
			None => Ok(()),
		}
	}
}

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
				grid: None,
				sprites: Sprites::default(),
			})),
		}
	}

	/// Use `sprites` to resolve sprite brushes. Call before [`install`](Self::install).
	pub fn set_sprites(&self, sprites: Sprites) {
		self.machine.borrow_mut().sprites = sprites;
	}

	/// Build the roster + sprite classes and the injected unit methods, then wire
	/// `World.getRoster`/`getPlayerRoster`/`createRoster`/`deleteRoster`. Call once with
	/// `&mut SQVM`, after the `World` table exists.
	pub fn install(&self, vm: &mut SQVM) {
		let sprites = self.machine.borrow().sprites.clone();
		self.machine.borrow_mut().sprite_class = Some(build_sprite_class(vm, &sprites));

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
		let is_placed_on_map = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM| -> bool { m.borrow().record(vm.this_identity()).is_some_and(|r| r.tile.is_some()) })
		};
		let get_tile = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM| -> Option<SqStrongRef> {
				let idx = m.borrow().by_identity.get(&vm.this_identity()).copied()?;
				placement::tile_of(&m, vm, idx)
			})
		};
		let is_actor = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM| -> bool { m.borrow().record(vm.this_identity()).is_some_and(|r| r.is_actor) })
		};
		// setTile(tile): stand on another tile (moved there directly, as a teleport).
		let set_tile = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM, tile: squirrel::Native<crate::native::tactical_grid::TileHandle>| -> Result<(), String> {
				let Some(idx) = m.borrow().by_identity.get(&vm.this_identity()).copied() else {
					return Ok(());
				};
				let is_actor = m.borrow().units[idx].is_actor;
				placement::unplace(&m, idx);
				placement::place(&m, idx, tile.0.id, is_actor)
			})
		};
		let get_relative_pos = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM| -> Result<Option<SqStrongRef>, String> {
				let (x, y) = m.borrow().record(vm.this_identity()).map_or((0.0, 0.0), |r| r.pos);
				let create_vec = SqTable::root(vm).get::<SqStrongRef>("createVec").ok_or("createVec missing")?;
				create_vec.call_ret::<SqStrongRef, _>(vm, (x, y))
			})
		};
		let insert_sprite = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM, name: String, Variadic(_rest): Variadic<Retained>| -> Option<SqStrongRef> {
				let class = m.borrow().sprite_class.clone()?;
				let sprite = class.instantiate(vm, UnitSpriteData::fresh());
				if let Some(rec) = m.borrow_mut().record_mut(vm.this_identity()) {
					rec.sprites.insert(name, sprite.clone());
				}
				Some(sprite)
			})
		};
		let remove_sprite = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM, name: String| {
				if let Some(rec) = m.borrow_mut().record_mut(vm.this_identity()) {
					rec.sprites.remove(&name);
				}
			})
		};
		// die(): the entity's onFinish runs (an actor's lifts its zones while still on
		// the map), then it leaves the map and the battle (probed 2026-10-13: right
		// after die() it is off the map, its tile empty, getEntityByID null).
		let die = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM| -> Result<(), String> {
				let idx = m.borrow().by_identity.get(&vm.this_identity()).copied();
				let Some(idx) = idx else { return Ok(()) };
				let entity = m.borrow().units[idx].entity.clone();
				ignore_missing(entity.call_method(vm, "onFinish", ()))?;
				placement::unplace(&m, idx);
				m.borrow_mut().units[idx].dead = true;
				Ok(())
			})
		};
		let remove_from_map = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM| -> Result<(), String> {
				let idx = m.borrow().by_identity.get(&vm.this_identity()).copied();
				match idx {
					Some(idx) => placement::remove_from_map(&m, vm, idx),
					None => Ok(()),
				}
			})
		};
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
		let (is_discovered, _) = flag(vm, &self.machine, |r| r.discovered, |r, v| r.discovered = v);
		let (is_blocking_sight, set_block_sight) =
			flag(vm, &self.machine, |r| r.blocks_sight, |r, v| r.blocks_sight = v);
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
			("isBlockingSight", is_blocking_sight),
			("setBlockSight", set_block_sight),
			("getPos", get_pos),
			("setPos", set_pos),
			("removeFromMap", remove_from_map),
			("die", die),
			("isActor", is_actor),
			("setTile", set_tile),
			("getRelativePos", get_relative_pos),
			("insertSprite", insert_sprite),
			("removeSprite", remove_sprite),
			("isStoringColor", vm.make_closure(|| false)),
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
			"clearVisibility",
			"setZ",
			"storeSpriteColors",
			"restoreSpriteColors",
			"fadeTo",
			"fadeToStoredColors",
			"setSpriteOcclusion",
			"moveSpriteScale",
			"setAlwaysApplySpriteOffset",
			"setSpriteRenderToTexture",
			"copySpritesFrom",
		] {
			methods.push((name, noop.clone()));
		}
		methods.extend(vision::unit_natives(vm, &self.machine));
		methods.extend(render::unit_natives(vm, &self.machine));
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
		let fixed_roster = |vm: &mut SQVM, id: i64| {
			let mint = mint_roster.clone();
			vm.make_closure(move |vm: &SQVM| -> Option<SqStrongRef> { mint(vm, id) })
		};
		let get_guest_roster = fixed_roster(vm, GUEST_ROSTER_ID);
		let get_casualty_roster = fixed_roster(vm, CASUALTY_ROSTER_ID);
		let get_survivor_roster = fixed_roster(vm, SURVIVOR_ROSTER_ID);
		let get_retreat_roster = fixed_roster(vm, RETREAT_ROSTER_ID);
		let get_temporary_roster = fixed_roster(vm, TEMPORARY_ROSTER_ID);
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
				let unit = machine.units.iter().find(|u| u.id == id && !u.dead)?;
				// On the battle map (spawned or added): the scene knows it.
				if unit.tile.is_some() {
					return Some(unit.entity.clone());
				}
				let identity = unit.entity.identity();
				machine
					.rosters
					.values()
					.flatten()
					.find(|e| e.identity() == identity)
					.cloned()
			})
		};
		// Tactical.addEntityToMap(entity, x, y) and the spawnEntity overloads (semantics
		// probed in the original, 2026-10-13; see `placement`).
		let add_entity_to_map = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM, entity: Retained, q: i64, r: i64| placement::add_to_map(&m, vm, &entity.0, q, r))
		};
		let spawn_tile = |m: &Rc<RefCell<TacticalMachine>>, q: i64, r: i64| -> Result<usize, String> {
			let grid = m.borrow().grid.clone().ok_or("spawnEntity: no battle map")?;
			let id = grid.borrow().idx_axial(q, r);
			id.ok_or_else(|| format!("spawnEntity: ({q}, {r}) is off the map"))
		};
		// spawnEntity(path) spawns at the tile under the cursor (a debug key); with none
		// the original crashes.
		let spawn_1 = vm.make_closure(|_path: String| -> Result<Option<SqStrongRef>, String> {
			Err("spawnEntity(path): spawns at the hovered tile, and there is none headless".to_string())
		});
		let spawn_2 = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM, path: String, coords: Retained| -> Result<Option<SqStrongRef>, String> {
				let q = coords.0.get_member::<squirrel::Number>(vm, "X").ok_or("spawnEntity: coords need X")?.0 as i64;
				let r = coords.0.get_member::<squirrel::Number>(vm, "Y").ok_or("spawnEntity: coords need Y")?.0 as i64;
				placement::spawn_at(&m, vm, &path, spawn_tile(&m, q, r)?, 0)
			})
		};
		let spawn_3 = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM, path: String, q: i64, r: i64| -> Result<Option<SqStrongRef>, String> {
				placement::spawn_at(&m, vm, &path, spawn_tile(&m, q, r)?, 0)
			})
		};
		let spawn_4 = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM, path: String, q: i64, r: i64, parent: i64| -> Result<Option<SqStrongRef>, String> {
				placement::spawn_at(&m, vm, &path, spawn_tile(&m, q, r)?, parent)
			})
		};
		let spawn_dispatch = make_dispatcher(vm, "spawnEntity");
		let vision_natives = vision::tactical_natives(vm, &self.machine);
		let render_natives = render::tactical_natives(vm, &self.machine);
		let effect_natives = effects::natives(vm, &self.machine);

		let mut world = SqTable::root(vm)
			.get::<SqTable>("World")
			.expect("World table must be installed before TacticalEntities");
		world.set("getRoster", get_roster);
		world.set("getPlayerRoster", get_player_roster);
		world.set("createRoster", create_roster);
		world.set("deleteRoster", delete_roster);
		world.set("getGuestRoster", get_guest_roster);
		let mut tactical = SqTable::root(vm)
			.get::<SqTable>("Tactical")
			.expect("Tactical table must be installed before TacticalEntities");
		tactical.set("getEntityByID", get_entity_by_id);
		tactical.set("addEntityToMap", add_entity_to_map);
		for (name, f) in vision_natives.into_iter().chain(effect_natives).chain(render_natives) {
			tactical.set(name, f);
		}
		for (arity, f) in [(1, spawn_1), (2, spawn_2), (3, spawn_3), (4, spawn_4)] {
			tactical.set(&overload_key("spawnEntity", arity), f);
		}
		tactical.set("spawnEntity", spawn_dispatch);
		tactical.set("getCasualtyRoster", get_casualty_roster);
		tactical.set("getSurvivorRoster", get_survivor_roster);
		tactical.set("getRetreatRoster", get_retreat_roster);
		tactical.set("getTemporaryRoster", get_temporary_roster);
	}

	/// Place units and objects on `grid`, and create the objects `tile.spawnObject`
	/// asks for.
	pub fn attach_map(&self, grid: &crate::native::TacticalGrid) {
		self.machine.borrow_mut().grid = Some(grid.handle());
		grid.set_object_spawner(placement::object_spawner(&self.machine));
		let m = self.machine.clone();
		grid.set_occupant_lookup(Rc::new(move || vision::occupants(&m)));
	}

	/// The navigator's handle on the units.
	pub(crate) fn units(&self) -> Units {
		Units(self.machine.clone())
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
		m.grid = None;
	}
}
