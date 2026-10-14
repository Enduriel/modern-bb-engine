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

use squirrel::{ClassBuilder, Number, Retained, SQVM, SqArray, SqClass, SqStrongRef, SqTable, Variadic};

use super::inject::inject_natives;
use super::overload::{make_dispatcher, overload_key, single_overload};

mod persist;
mod colors;
mod effects;
mod original;
mod overlay;
mod placement;
mod render;
mod shaker;
mod vision;
use crate::module::Instantiator;
use crate::native::spin::Spin;
use crate::native::sprite_anim::Anim;
use crate::native::sprite_class::{self, Flag, Kind, Num, SpriteHandle, SpriteState};
use crate::native::sprite_color::{SpriteColor, real_time};
use crate::native::{EntityIds, Sprites};

/// The player company roster's id (real faction/settlement roster ids are >= 0).
const PLAYER_ROSTER_ID: i64 = -1;
/// The other fixed rosters: the world's guests (`World.getGuestRoster`), and a battle's
/// casualties, survivors, retreated units and temporary units (`Tactical.get…Roster`).
const GUEST_ROSTER_ID: i64 = -2;
const CASUALTY_ROSTER_ID: i64 = -3;
const SURVIVOR_ROSTER_ID: i64 = -4;
const RETREAT_ROSTER_ID: i64 = -5;
const TEMPORARY_ROSTER_ID: i64 = -6;
/// The world's own temporary roster (`World.getTemporaryRoster`), where events and
/// contracts put the people they offer; a different roster from the battle's
/// (disassembled: `World+0x26c` vs `Tactical+0x2cc`).
const WORLD_TEMPORARY_ROSTER_ID: i64 = -7;
/// The id `World.getRoster`/`createRoster`/`deleteRoster` take for a non-number. The
/// original's bindings don't check the argument: an id that isn't a number reads as
/// whatever their stack slot held, the VM's address (disassembled, `0x4f8a00`), so every
/// such id names the same roster. Mods rely on it (Clever Recruiter keys its roster by
/// its mod id string). A large positive stand-in, as an address is.
const NON_NUMERIC_ROSTER_ID: i64 = 0x7f00_0000;

/// A roster id argument as the original's bindings read it (see [`NON_NUMERIC_ROSTER_ID`]).
fn roster_id(vm: &SQVM, id: &Retained) -> i64 {
	id.0.get_value::<Number>(vm).map_or(NON_NUMERIC_ROSTER_ID, |n| n.0 as i64)
}

/// A render sprite handle for a tactical unit. Richer than the world sprite (units read
/// more back): also `Rotation`, `Color`, `HasBrush`, `getBrush` and
/// `setHorizontalFlipping`. Its colour and fades are a [`SpriteColor`], as the world's
/// sprites' are, and `Tactical.render()` draws them (see `render`).
#[derive(Clone)]
struct UnitSpriteData {
	brush: String,
	/// `Color` (ARGB; `Alpha` is its top byte) and its fade. Skeletons copy their body's
	/// into their blood colour.
	color: SpriteColor,
	/// From the unit's position, in world units with y up (`setSpriteOffset`, the
	/// sprite's `setOffset`).
	offset: (f64, f64),
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
	/// `Highlight` (ARGB), blended over the sprite by `HighlightFactor`: a hit's flash.
	highlight: u32,
	highlight_factor: f64,
	/// The colours its slot keeps to put back (see `shaker`), and whether they're kept
	/// now.
	stored: shaker::Stored,
	is_stored: bool,
	/// Its `Torque` (see `spin`).
	spin: Spin,
	/// The frame of its brush it shows (see `sprite_anim`).
	anim: Anim,
	ignore_ambient: bool,
	ignore_camera_flip: bool,
	/// `setSpriteOcclusion(name, a, b, c)`, kept on the sprite's slot (`0x4a8950`):
	/// trees' and boulders' tops, which `HideTrees` fades.
	occlusion: Option<(i64, i64, i64)>,
	/// Drawn into the unit's portrait (`setSpriteRenderToTexture`, on by default).
	in_portrait: bool,
	/// `setSpriteColorization(name, on)` (`0x4a8a50`, the slot's byte `+0x1c`, on by
	/// default): off, the unit's colour natives (see `colors`) and its shakes leave the
	/// sprite alone, and its portrait draws it as it is (actors' `arrow`).
	colorization: bool,
	/// The flag bits its brush calls left (see [`Sprites::brush_bits`]), which the
	/// original saves.
	brush_bits: u32,
}

impl UnitSpriteData {
	fn fresh() -> Self {
		UnitSpriteData {
			brush: String::new(),
			color: SpriteColor::default(),
			offset: (0.0, 0.0),
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
			highlight: 0,
			highlight_factor: 0.0,
			stored: shaker::Stored::default(),
			is_stored: false,
			spin: Spin::default(),
			anim: Anim::default(),
			ignore_ambient: false,
			ignore_camera_flip: false,
			occlusion: None,
			in_portrait: true,
			colorization: true,
			brush_bits: 0,
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
	/// The portrait draws the sprites at their offsets (`setAlwaysApplySpriteOffset`;
	/// scripts set it in `onInit`).
	portrait_offsets: bool,
	/// The bars over a unit and its status icons (`createOverlay`; see `overlay`).
	overlay: Option<overlay::Overlay>,
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
	/// Named sprites, in the order they were added (their layering).
	sprites: super::sprite_slots::Slots,
	/// A shake keeps some of its sprites' colours (`entity+0x50`; see `shaker`).
	colors_stored: bool,
	/// In the air between two tiles (the original's flight layer, flag `0x800000`).
	flying: bool,
	/// Drawn on a tile the player can't see now (`setVisibleInFogOfWar`, `+0x18b`;
	/// true until actors turn it off).
	visible_in_fog: bool,
	/// `setPreventOcclusion` (`0x4c2bf0`, `+0x188`; every actor turns it on): trees and
	/// boulders in front of its tile fade (see `tactical_unveil`).
	prevent_occlusion: bool,
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
	/// Unit ids, shared with the world's entities (see `entity_ids`).
	ids: EntityIds,
	methods: Vec<(&'static str, SqStrongRef)>,
	/// The battle map, for placing units and objects.
	grid: Option<crate::native::tactical_grid::Grid>,
	/// Resolves sprite brushes (set by the host before `install`).
	sprites: Sprites,
	/// The battle's visual effects (see `effects`).
	effects: Option<crate::native::battle_effects::BattleEffectsHandle>,
	/// The units shaking now, in the order they started (see `shaker`).
	shakes: Vec<shaker::Shake>,
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
			overlay: None,
			portrait_offsets: false,
			discovered: false,
			blocks_sight: false,
			pos: (0.0, 0.0),
			tile: None,
			is_actor,
			dead: false,
			render_callback: false,
			sprites: super::sprite_slots::Slots::default(),
			colors_stored: false,
			flying: false,
			visible_in_fog: true,
			prevent_occlusion: false,
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
	let id = machine.borrow().ids.next();
	let (entity, record_idx) = create_unit(machine, vm, path, id)?;
	entity.call_method(vm, "onInit", ())?;
	ignore_missing(entity.call_method(vm, "onAfterInit", ()))?;
	// The one-time onInit setup has run; the entity is now initialized.
	machine.borrow_mut().units[record_idx].initialized = true;
	Ok(entity)
}

impl SpriteState for UnitSpriteData {
	fn color(&self) -> &SpriteColor {
		&self.color
	}
	fn color_mut(&mut self) -> &mut SpriteColor {
		&mut self.color
	}
	fn anim(&self) -> &Anim {
		&self.anim
	}
	fn anim_mut(&mut self) -> &mut Anim {
		&mut self.anim
	}
	fn spin(&self) -> &Spin {
		&self.spin
	}
	fn spin_mut(&mut self) -> &mut Spin {
		&mut self.spin
	}
	fn brush(&self) -> &str {
		&self.brush
	}
	fn store_brush(&mut self, sprites: &Sprites, brush: String) {
		self.brush_bits = sprites.brush_bits(&brush, self.brush_bits);
		self.brush = brush;
	}
	fn set_offset(&mut self, x: f64, y: f64) {
		self.offset = (x, y);
	}
	fn num(&self, n: Num) -> f64 {
		match n {
			Num::Scale => self.scale,
			Num::Rotation => self.rotation,
			Num::Saturation => self.saturation,
			Num::HighlightFactor => self.highlight_factor,
		}
	}
	fn set_num(&mut self, n: Num, v: f64) {
		match n {
			Num::Scale => self.scale = v,
			Num::Rotation => self.rotation = v,
			Num::Saturation => self.saturation = v,
			Num::HighlightFactor => self.highlight_factor = v,
		}
	}
	fn flag(&self, f: Flag) -> bool {
		match f {
			Flag::Visible => self.visible,
			Flag::Flipped => self.flipped,
			Flag::IgnoreAmbientColor => self.ignore_ambient,
			Flag::IgnoreCameraFlip => self.ignore_camera_flip,
		}
	}
	fn set_flag(&mut self, f: Flag, on: bool) {
		match f {
			Flag::Visible => self.visible = on,
			Flag::Flipped => self.flipped = on,
			Flag::IgnoreAmbientColor => self.ignore_ambient = on,
			Flag::IgnoreCameraFlip => self.ignore_camera_flip = on,
		}
	}
	fn highlight(&self) -> u32 {
		self.highlight
	}
	fn set_highlight(&mut self, argb: u32) {
		self.highlight = argb;
	}
}

impl SpriteHandle for UnitSpriteData {
	fn with<R>(&self, f: impl FnOnce(&dyn SpriteState) -> R) -> R {
		f(self)
	}
	fn with_mut<R>(&mut self, f: impl FnOnce(&mut dyn SpriteState) -> R) -> R {
		f(self)
	}
}

/// Build the unit sprite class: the sprite class (see `sprite_class`), and the weapon
/// and shield animation switches.
fn build_sprite_class(vm: &mut SQVM, sprites: &Sprites) -> SqClass<UnitSpriteData> {
	let b = ClassBuilder::<UnitSpriteData>::new("TacticalUnitSprite");
	sprite_class::members(b, Kind { sprites: sprites.clone(), battle: true, moving: None })
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
		.method(
			"setRenderCallbackEnabled",
			|_s: &mut UnitSpriteData, _a: Variadic<Retained>| crate::native::stubs::note("Battle unit sprite.setRenderCallbackEnabled"),
		)
		.build(vm)
}

/// Build the roster handle class (`getSize`/`getAll`/`get`/`create`/`add`/`remove`/`clear`).
fn build_roster_class(
	vm: &mut SQVM,
	machine: &Rc<RefCell<TacticalMachine>>,
) -> SqClass<RosterData> {
	let m_size = machine.clone();
	let m_all = machine.clone();
	let m_get = machine.clone();
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
		// get(i): the i-th member; past the end (or negative), an empty table (probed;
		// disassembled, 0x4b17d0).
		.method("get", move |r: &mut RosterData, vm: &SQVM, index: i64| -> SqStrongRef {
			let member = {
				let m = m_get.borrow();
				let members = m.rosters.get(&r.id);
				usize::try_from(index).ok().and_then(|i| members.and_then(|v| v.get(i))).cloned()
			};
			member.unwrap_or_else(|| SqTable::new(vm).to_strong())
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

	/// Show the unit at `pos` (world units, y up), where a move has it now.
	pub fn set_pos(&self, identity: i64, pos: (f64, f64)) {
		if let Some(r) = self.0.borrow_mut().record_mut(identity) {
			r.pos = pos;
		}
	}

	/// The unit is in the air between two tiles, or back on one.
	pub fn set_flying(&self, identity: i64, flying: bool) {
		if let Some(r) = self.0.borrow_mut().record_mut(identity) {
			r.flying = flying;
		}
	}

	/// End the unit's shake (`Tactical.getShaker().cancel(unit)`).
	pub fn cancel_shake(&self, vm: &SQVM, identity: i64) {
		shaker::cancel(&self.0, vm, identity);
	}

	/// Fade all the unit's sprites in, or out, over `ms` of real time (the entity's
	/// `0x4a9930`, `0x4a99c0`).
	pub fn fade(&self, vm: &SQVM, identity: i64, ms: i64, out: bool) {
		let now = real_time(vm);
		self.each_sprite(vm, identity, |s| if out { s.color.fade_out(ms, now, false) } else { s.color.fade_in(ms, now) });
	}

	/// All the unit's sprites fully opaque, their fades dropped (`0x4a9a50`).
	pub fn opaque(&self, vm: &SQVM, identity: i64) {
		self.each_sprite(vm, identity, |s| s.color.snap_alpha(255));
	}

	fn each_sprite(&self, vm: &SQVM, identity: i64, mut f: impl FnMut(&mut UnitSpriteData)) {
		let m = self.0.borrow();
		let (Some(class), Some(r)) = (m.sprite_class.clone(), m.record(identity)) else { return };
		for (_, h) in r.sprites.iter() {
			class.with_payload(vm, h, &mut f);
		}
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

#[derive(Clone)]
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
				ids: EntityIds::default(),
				methods: Vec::new(),
				grid: None,
				sprites: Sprites::default(),
				effects: None,
				shakes: Vec::new(),
			})),
		}
	}

	/// Use `sprites` to resolve sprite brushes. Call before [`install`](Self::install).
	/// Draw unit ids from `ids` (the world's counter): one id space, as the original's.
	pub fn share_ids(&self, ids: EntityIds) {
		self.machine.borrow_mut().ids = ids;
	}

	pub fn set_sprites(&self, sprites: Sprites) {
		self.machine.borrow_mut().sprites = sprites;
	}

	/// Where the `Tactical.spawn…Effect` natives put their effects. Call before
	/// [`install`](Self::install).
	pub(crate) fn set_battle_effects(&self, effects: crate::native::battle_effects::BattleEffectsHandle) {
		self.machine.borrow_mut().effects = Some(effects);
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
		// addSprite(name) / insertSprite(name): see `new_sprite`.
		let add_sprite = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM, name: String| new_sprite(&m, vm, name, false))
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
		// createOverlay(): the unit gets its bars (empty until setOverlayValues). It also
		// returns an inert sprite, which no script reads.
		let create_overlay = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM, _a: Variadic<Retained>| -> Option<SqStrongRef> {
				if let Some(rec) = m.borrow_mut().record_mut(vm.this_identity()) {
					rec.overlay.get_or_insert_with(Default::default);
				}
				let class = m.borrow().sprite_class.clone()?;
				Some(class.instantiate(vm, UnitSpriteData::fresh()))
			})
		};
		// setOverlayValues(headArmor, bodyArmor, hitpoints): what the bars show.
		let set_overlay_values = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM, head: Number, body: Number, hitpoints: Number| {
				if let Some(rec) = m.borrow_mut().record_mut(vm.this_identity()) {
					if let Some(overlay) = rec.overlay.as_mut() {
						overlay.set_values([head.0 as f32, body.0 as f32, hitpoints.0 as f32], real_time(vm));
					}
				}
			})
		};
		// setOverlayIcons(brushes): the icons over a unit's bars (`0x4b42f0`); nothing
		// without bars or for anything but an array.
		let set_overlay_icons = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM, icons: Retained| {
				let Some(icons) = icons.0.get_value::<SqArray>(vm) else { return };
				let icons: Vec<String> = (0..icons.len()).filter_map(|i| icons.get::<String>(i)).collect();
				if let Some(rec) = m.borrow_mut().record_mut(vm.this_identity()) {
					if let Some(overlay) = rec.overlay.as_mut() {
						overlay.icons = icons;
					}
				}
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
			vm.make_closure(move |vm: &SQVM, name: String, Variadic(_rest): Variadic<Retained>| {
				new_sprite(&m, vm, name, true)
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
		// getSpriteOffset(name) / setSpriteOffset(name, offset): the named sprite's offset
		// (a zero vector for an unknown name).
		let get_sprite_offset = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM, name: String| -> Result<Option<SqStrongRef>, String> {
				let (handle, class) = {
					let m = m.borrow();
					(m.record(vm.this_identity()).and_then(|r| r.sprites.get(&name).cloned()), m.sprite_class.clone())
				};
				let offset = match (handle, class) {
					(Some(h), Some(c)) => c.with_payload(vm, &h, |s| s.offset).unwrap_or((0.0, 0.0)),
					_ => (0.0, 0.0),
				};
				let create_vec = SqTable::root(vm)
					.get::<SqStrongRef>("createVec")
					.ok_or("getSpriteOffset: createVec missing")?;
				create_vec.call_ret::<SqStrongRef, _>(vm, offset)
			})
		};
		// setSpriteRenderToTexture(name, on) and setAlwaysApplySpriteOffset(on): what the
		// unit's portrait draws (see `portrait`).
		let set_sprite_in_portrait = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM, name: String, on: bool| {
				let (handle, class) = {
					let m = m.borrow();
					(m.record(vm.this_identity()).and_then(|r| r.sprites.get(&name).cloned()), m.sprite_class.clone())
				};
				if let (Some(h), Some(class)) = (handle, class) {
					class.with_payload(vm, &h, |s| s.in_portrait = on);
				}
			})
		};
		let set_portrait_offsets = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM, on: bool| {
				if let Some(rec) = m.borrow_mut().record_mut(vm.this_identity()) {
					rec.portrait_offsets = on;
				}
			})
		};
		// setSpriteOcclusion(name, a, b, c): see `UnitSpriteData::occlusion`.
		let set_sprite_occlusion = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM, name: String, a: i64, b: i64, c: i64| {
				let (handle, class) = {
					let m = m.borrow();
					(m.record(vm.this_identity()).and_then(|r| r.sprites.get(&name).cloned()), m.sprite_class.clone())
				};
				if let (Some(h), Some(class)) = (handle, class) {
					class.with_payload(vm, &h, |s| s.occlusion = Some((a, b, c)));
				}
			})
		};
		// setSpriteColorization(name, on): see `UnitSpriteData::colorization`.
		let set_sprite_colorization = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM, name: String, on: bool| {
				let (handle, class) = {
					let m = m.borrow();
					(m.record(vm.this_identity()).and_then(|r| r.sprites.get(&name).cloned()), m.sprite_class.clone())
				};
				if let (Some(h), Some(class)) = (handle, class) {
					class.with_payload(vm, &h, |s| s.colorization = on);
				}
			})
		};
		let set_prevent_occlusion = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM, on: bool| {
				if let Some(rec) = m.borrow_mut().record_mut(vm.this_identity()) {
					rec.prevent_occlusion = on;
				}
			})
		};
		let set_sprite_offset = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM, name: String, offset: Retained| {
				let (handle, class) = {
					let m = m.borrow();
					(m.record(vm.this_identity()).and_then(|r| r.sprites.get(&name).cloned()), m.sprite_class.clone())
				};
				if let (Some(h), Some(c)) = (handle, class) {
					let c2 = |axis: &str| offset.0.get_member::<Number>(vm, axis).map_or(0.0, |n| n.0);
					let xy = (c2("X"), c2("Y"));
					c.with_payload(vm, &h, |s| s.offset = xy);
				}
			})
		};
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
			("setSpriteOffset", set_sprite_offset),
			("setSpriteOcclusion", set_sprite_occlusion),
			("setSpriteColorization", set_sprite_colorization),
			("setPreventOcclusion", set_prevent_occlusion),
			("setSpriteRenderToTexture", set_sprite_in_portrait),
			("setAlwaysApplySpriteOffset", set_portrait_offsets),
			("createOverlay", create_overlay),
			("setOverlayValues", set_overlay_values),
			("setOverlayIcons", set_overlay_icons),
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
		];
		for name in [
			"setDirty",
			"setVisibility",
			"setSpriteScaling",
			"addDefaultStatusSprites",
			"updateOverlay",
			// A bare `ret` in the original.
			"clearVisibility",
			// Only stores an int the original doesn't read back (`entity+0x10`).
			"setZ",
		] {
			methods.push((name, crate::native::stubs::closure(vm, crate::native::stubs::name(format!("Battle unit.{name}")))));
		}
		// One-arity Sqrat overloads in the original (see `colors`).
		let (store, restore) = colors::store_and_restore(vm, &self.machine);
		for (name, key, leaf) in [
			("storeSpriteColors", "__sqrat_ol_ storeSpriteColors_0", store),
			("restoreSpriteColors", "__sqrat_ol_ restoreSpriteColors_0", restore),
		] {
			methods.extend(single_overload(vm, name, key, leaf));
		}
		methods.extend(colors::unit_natives(vm, &self.machine));
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
			vm.make_closure(move |vm: &SQVM, id: Retained| -> Option<SqStrongRef> { mint(vm, roster_id(vm, &id)) })
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
		let get_world_temporary_roster = fixed_roster(vm, WORLD_TEMPORARY_ROSTER_ID);
		// getAllRosters(): every created roster (factions', settlements'), by id. The
		// fixed ones (player, guests, temporary) aren't among them (disassembled: the
		// world keeps them apart from its roster map, 0x4f4340).
		let get_all_rosters = {
			let mint = mint_roster.clone();
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM| -> SqStrongRef {
				let mut ids: Vec<i64> = m.borrow().rosters.keys().copied().filter(|&id| id >= 0).collect();
				ids.sort_unstable();
				let mut list = SqArray::new(vm);
				for id in ids {
					if let Some(r) = mint(vm, id) {
						list.push(r);
					}
				}
				list.to_strong()
			})
		};
		// createRoster(id): the roster with that id, made if there is none (disassembled,
		// 0x4f43d0); Nested Tooltips keeps its dummy player in one.
		let create_roster = {
			let m = self.machine.clone();
			let mint = mint_roster.clone();
			vm.make_closure(move |vm: &SQVM, id: Retained| -> Option<SqStrongRef> {
				let id = roster_id(vm, &id);
				m.borrow_mut().rosters.entry(id).or_default();
				mint(vm, id)
			})
		};
		let delete_roster = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM, id: Retained| {
				m.borrow_mut().rosters.remove(&roster_id(vm, &id));
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
		world.set("getTemporaryRoster", get_world_temporary_roster);
		world.set("getAllRosters", get_all_rosters);
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

	/// The units and objects on the battle map, with their sprites in layering order,
	/// as a renderer reads them.
	/// Turn the placed units' sprites that have a torque to virtual time `now`, in
	/// seconds (see `spin`), and step their animations to virtual millisecond `now_ms`
	/// (see `sprite_anim`).
	pub(crate) fn turn(&self, vm: &SQVM, now: f32, now_ms: u32) {
		let m = self.machine.borrow();
		let Some(class) = m.sprite_class.clone() else { return };
		for u in m.units.iter().filter(|u| !u.dead && u.tile.is_some()) {
			for (_, handle) in u.sprites.iter() {
				class.with_payload(vm, handle, |s| {
					s.rotation = f64::from(s.spin.turn(s.rotation as f32, now));
					if let Some(scale) = s.anim.step_on(&m.sprites, &s.brush, now_ms) {
						s.scale = f64::from(scale);
					}
				});
			}
		}
	}

	/// How a sprite a script holds looks now (a head's layer, see `battle_effects`).
	pub(crate) fn sprite_look(&self, vm: &SQVM, handle: &SqStrongRef) -> Option<crate::native::battle_effects::LayerLook> {
		let class = self.machine.borrow().sprite_class.clone()?;
		class
			.with_payload(vm, handle, |s| crate::native::battle_effects::LayerLook {
				brush: s.brush.clone(),
				color: s.color.shown(real_time(vm)),
				saturation: s.saturation as f32,
				scale: s.scale as f32,
				flipped: s.flipped,
			})
			.filter(|l| !l.brush.is_empty())
	}

	pub(crate) fn scene_entities(&self, vm: &SQVM) -> Vec<crate::scene::tactical::SceneTacticalEntity> {
		let now = real_time(vm);
		let shown = overlay::Shown {
			always: crate::native::temp_gameplay_flag(vm, "ShowOverlayStats"),
			// Gameplay byte 3, which scripts can't see (see `overlay`): on.
			after_change: true,
		};
		let m = self.machine.borrow();
		let Some(class) = m.sprite_class.clone() else { return Vec::new() };
		m.units
			.iter()
			.filter(|u| !u.dead && u.tile.is_some())
			.map(|u| crate::scene::tactical::SceneTacticalEntity {
				id: u.id,
				pos: u.pos,
				tile: u.tile,
				flying: u.flying,
				visible_in_fog: u.visible_in_fog,
				is_actor: u.is_actor,
				overlay: u.overlay.as_ref().and_then(|o| o.scene(now, shown)),
				sprites: u
					.sprites
					.iter()
					.filter_map(|(name, handle)| {
						class.with_payload(vm, handle, |s| crate::scene::tactical::SceneTacticalSprite {
							name: name.clone(),
							brush: s.brush.clone(),
							color: s.color.shown(now),
							scale: s.scale as f32,
							rotation: s.rotation as f32,
							saturation: s.saturation as f32,
							highlight: s.highlight,
							highlight_factor: s.highlight_factor as f32,
							visible: s.visible,
							flipped: s.flipped,
							offset: (s.offset.0 as f32, s.offset.1 as f32),
							ignore_ambient: s.ignore_ambient,
							occlusion: s.occlusion,
							frame: s.anim.frame(),
						})
					})
					.collect(),
			})
			.collect()
	}

	/// What unit `id`'s portrait draws, placed on the map or not (`None` for no such
	/// unit).
	pub(crate) fn portrait(&self, vm: &SQVM, id: i64) -> Option<crate::scene::ScenePortrait> {
		let m = self.machine.borrow();
		let class = m.sprite_class.clone()?;
		let unit = m.units.iter().find(|u| u.id == id && !u.dead)?;
		// A colourised sprite is drawn with its kept colours while the unit keeps them,
		// else at full alpha: a faded or shaken unit's portrait shows it as it was
		// (`0x4a8700`).
		let portrait_look = |s: &UnitSpriteData| match (s.colorization, unit.colors_stored && s.is_stored) {
			(false, _) => (s.color.color, s.saturation),
			(true, true) => (s.stored.color, s.stored.saturation),
			(true, false) => (s.color.color | 0xff00_0000, s.saturation),
		};
		let sprites = unit
			.sprites
			.iter()
			.filter_map(|(name, handle)| {
				class.with_payload(vm, handle, |s| crate::scene::PortraitSprite {
					name: name.clone(),
					brush: s.brush.clone(),
					color: portrait_look(s).0,
					scale: s.scale as f32,
					rotation: s.rotation as f32,
					saturation: portrait_look(s).1 as f32,
					flipped: s.flipped,
					offset: (s.offset.0 as f32, s.offset.1 as f32),
					visible: s.visible,
					in_portrait: s.in_portrait,
				})
			})
			.collect();
		Some(crate::scene::ScenePortrait {
			sprites,
			apply_offsets: unit.portrait_offsets,
		})
	}

	/// The tiles of the live units that prevent occlusion (see `tactical_unveil`).
	pub(crate) fn occlusion_preventers(&self) -> Vec<usize> {
		let m = self.machine.borrow();
		m.units.iter().filter(|u| !u.dead && u.prevent_occlusion).filter_map(|u| u.tile).collect()
	}

	/// The shaker (`Tactical.getShaker()`), over the units. Call after the navigator and
	/// the battle map are installed: it runs after their `Tactical.update()` and before
	/// their `Tactical.clearScene()`.
	pub fn install_shaker(&self, vm: &mut SQVM) {
		shaker::install(&self.machine, vm);
	}

	/// Total units created (host/test inspection).
	pub fn unit_count(&self) -> usize {
		self.machine.borrow().units.len()
	}

	/// The unit records kept for units that died (host/test inspection).
	pub fn dead_unit_count(&self) -> usize {
		self.machine.borrow().units.iter().filter(|u| u.dead).count()
	}

	/// Release every retained ref (rosters, units, their sprites, the classes, methods).
	/// Call before the VM is closed.
	pub fn clear(&self) {
		let mut m = self.machine.borrow_mut();
		m.shakes.clear();
		m.rosters.clear();
		m.units.clear();
		m.by_identity.clear();
		m.methods.clear();
		m.roster_class = None;
		m.sprite_class = None;
		m.grid = None;
	}
}

/// `addSprite(name)` (`first`: `insertSprite`) on the calling unit, as the original's
/// entity base class (`0x4a7dc0`, `0x4a7a60`): a new sprite on top of the others, or
/// below them all; a name the unit has gets that sprite back (the original logs that it
/// already exists).
fn new_sprite(m: &Rc<RefCell<TacticalMachine>>, vm: &SQVM, name: String, first: bool) -> Option<SqStrongRef> {
	let class = {
		let machine = m.borrow();
		if let Some(there) = machine.record(vm.this_identity()).and_then(|rec| rec.sprites.get(&name)) {
			return Some(there.clone());
		}
		machine.sprite_class.clone()?
	};
	let sprite = class.instantiate(vm, UnitSpriteData::fresh());
	if let Some(rec) = m.borrow_mut().record_mut(vm.this_identity()) {
		if first {
			rec.sprites.insert_first(name, sprite.clone());
		} else {
			rec.sprites.insert(name, sprite.clone());
		}
	}
	Some(sprite)
}
