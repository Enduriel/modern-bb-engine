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
//! ## Where the natives go
//!
//! Like the original engine, the natives are injected onto the instance's **base**
//! class-level table (bound to the leaf), and every level's identity is registered
//! against the entity's record; see [`super::inject`] and [`crate::class`].
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

use squirrel::{ClassBuilder, Number, Retained, SQVM, SqArray, SqBytes, SqClass, SqStrongRef, SqTable, Variadic};

use super::inject::inject_natives;
use super::overload::{make_dispatcher, overload_key};
use super::world_layout;
use super::world_query::{self, QueryEntity};
use crate::module::Instantiator;
use crate::native::{EntityIds, Sprites};
use crate::native::sprite_color::{SpriteColor, real_time};

mod footprints;
mod movement;
mod persist;
mod render;
use super::sprite_slots as slots;
mod sprites;
mod think;
mod view;

/// A render sprite handle (`entity.addSprite(name)`): what scripts set on it, which
/// the renderer reads through the world scene.
#[derive(Clone)]
struct SpriteData {
	brush: String,
	/// `Color` (and `Alpha`), with any fade in progress.
	color: SpriteColor,
	scale: f64,
	visible: bool,
	ignore_ambient: bool,
	/// From the entity's position, in world units with y up (`setOffset`,
	/// `setSpriteOffset`).
	offset: (f64, f64),
	/// Mirrored left to right (`setHorizontalFlipping`).
	flipped: bool,
	/// `setSpriteScaling(name, false)` turns it off.
	scaling: bool,
	/// `setSpriteColorization(name, false)` turns it off.
	colorization: bool,
	/// Drawn into the entity's portrait (`setSpriteRenderToTexture`, on by default).
	in_portrait: bool,
}

impl SpriteData {
	fn fresh() -> Self {
		SpriteData {
			brush: String::new(),
			color: SpriteColor::default(),
			scale: 1.0,
			visible: true,
			ignore_ambient: false,
			offset: (0.0, 0.0),
			flipped: false,
			scaling: true,
			colorization: true,
			in_portrait: true,
		}
	}
}

/// A text label (`entity.addLabel(name)`): what scripts set on it, which the
/// renderer reads through the world scene.
#[derive(Clone)]
struct LabelData {
	/// Bytes, not necessarily UTF-8: the original draws a label one glyph per byte.
	text: Vec<u8>,
	visible: bool,
	/// ARGB (`Color`, set from a `color` object).
	color: u32,
	/// From the entity's position, in world units with y up (`Offset`).
	offset: (f64, f64),
}

impl Default for LabelData {
	fn default() -> Self {
		LabelData {
			text: Vec::new(),
			visible: false,
			color: 0xffff_ffff,
			offset: (0.0, 0.0),
		}
	}
}

/// One registered world entity: its native id, the instance, its tile coords (axial),
/// and its named sprite/label handles. Reached from any of its class levels' identities
/// via [`EntityMachine::by_identity`].
struct EntityRecord {
	id: i64,
	/// The script it was created from (to re-create it on load).
	path: String,
	entity: SqStrongRef,
	q: i64,
	r: i64,
	/// World pixel position (a tile centre until `setPos` moves it).
	pos: (f64, f64),
	/// `setRenderedTop`: true by default (parties); locations clear it in `onInit`.
	/// Decides which half of the spatial queries finds the entity (see `world_query`).
	rendered_top: bool,
	/// Multiplier on the radius it can be seen from (`get/setVisibility`, default 1.0).
	visibility: f32,
	visible: bool,
	discovered: bool,
	visible_in_fog_of_war: bool,
	ignore_collision: bool,
	important: bool,
	z_offset: i64,
	think: think::ThinkState,
	render: render::RenderState,
	faction: i64,
	/// Cleared by `die`/`fadeAndDie` (which are idempotent, like the original's).
	alive: bool,
	/// When a `fadeAndDie` lets it die (real-time ms).
	dies_at: Option<i64>,
	/// The portrait draws the sprites at their offsets (`setAlwaysApplySpriteOffset`).
	portrait_offsets: bool,
	sprites: slots::Slots,
	labels: HashMap<String, SqStrongRef>,
}

/// Shared entity host state: the registry, the `this.new` capability, the stub classes,
/// the id counter, and the shared native method closures to inject onto each instance.
struct EntityMachine {
	instantiate: Instantiator,
	sprite_class: Option<SqClass<SpriteData>>,
	label_class: Option<SqClass<LabelData>>,
	/// Entity ids, shared with the battle's units (see `entity_ids`).
	ids: EntityIds,
	entities: Vec<EntityRecord>,
	/// Every class-level identity of every entity → its record index in `entities`.
	by_identity: HashMap<i64, usize>,
	/// Entity id → its record index in `entities` (`getEntityByID`).
	by_id: HashMap<i64, usize>,
	methods: Vec<(&'static str, SqStrongRef)>,
	player_view: view::PlayerView,
	footprints: footprints::Footprints,
	thinkers: think::Thinkers,
	/// Resolves sprite brushes (set by the host before `install`).
	sprites: Sprites,
}

impl EntityMachine {
	/// The live entities as query candidates, with their instances at the same indices.
	fn record(&self, identity: i64) -> Option<&EntityRecord> {
		self.by_identity.get(&identity).map(|&i| &self.entities[i])
	}
	fn record_mut(&mut self, identity: i64) -> Option<&mut EntityRecord> {
		let i = *self.by_identity.get(&identity)?;
		Some(&mut self.entities[i])
	}
}

/// Whether the world scene is shown (`World.isVisible()`). The world state hides it
/// while a battle (or a loading screen) is up, and then the world scene doesn't run:
/// no entity thinks or is discovered. Probed (2026-10-14): during a battle started
/// from the map, with the world unpaused at speed 1, no world party ran `onUpdate`
/// in 900 frames. The original's scene update and render both check the scene's
/// flags first.
pub(super) fn scene_shown(vm: &SQVM) -> Result<bool, String> {
	let world = SqTable::root(vm).get::<SqStrongRef>("World").ok_or("World missing")?;
	Ok(world.call_method_ret::<bool, _>(vm, "isVisible", ())?.unwrap_or(true))
}

/// Ignore a "no method" error (an optional hook the entity didn't define), but
/// propagate a real error raised inside one that does.
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
	for identity in inject_natives(vm, &leaf, &methods) {
		machine.borrow_mut().by_identity.insert(identity, record_idx);
	}
}

/// Instantiate `path` (runs `create()`) as entity `id` at axial `(q, r)`, and inject
/// the native methods (base class level, bound to the leaf). Returns the instance and
/// its record index. Runs no other script.
fn create_entity(
	machine: &Rc<RefCell<EntityMachine>>,
	vm: &SQVM,
	path: &str,
	id: i64,
	q: i64,
	r: i64,
) -> Result<(SqStrongRef, usize), String> {
	let instantiate = machine.borrow().instantiate.clone();
	let entity = instantiate(vm, path)?;
	let record_idx = {
		let mut m = machine.borrow_mut();
		let idx = m.entities.len();
		m.by_id.insert(id, idx);
		m.entities.push(EntityRecord {
			id,
			path: path.to_string(),
			entity: entity.clone(),
			q,
			r,
			pos: world_layout::tile_pos(q, r + q / 2),
			// The original's constructor defaults.
			rendered_top: true,
			visibility: 1.0,
			visible: true,
			discovered: false,
			visible_in_fog_of_war: false,
			ignore_collision: false,
			important: false,
			z_offset: 0,
			think: think::ThinkState::default(),
			render: render::RenderState::default(),
			faction: 0, // Neutral until settlement.onBuild/addFaction assigns one
			alive: true,
			dies_at: None,
			portrait_offsets: false,
			sprites: slots::Slots::default(),
			labels: HashMap::new(),
		});
		idx
	};
	inject_levels(machine, vm, &entity, record_idx);
	Ok((entity, record_idx))
}

/// Spawn a new entity of `path` at axial `(q, r)`: create it with the next id, then run
/// `onInit` + `onAfterInit`. No machine borrow is held across the script-running calls
/// (they re-enter the host).
fn spawn(
	machine: &Rc<RefCell<EntityMachine>>,
	vm: &SQVM,
	path: &str,
	q: i64,
	r: i64,
	occupy: bool,
) -> Result<SqStrongRef, String> {
	let id = machine.borrow().ids.next();
	let (entity, _) = create_entity(machine, vm, path, id, q, r)?;
	// Like the original, a spawned location occupies its tile (cleared again by the
	// script's `onFinish` when it dies); a spawned party does not (probed 2026-10-13).
	if occupy {
		set_tile_occupied(vm, q, r, true)?;
	}
	entity.call_method(vm, "onInit", ())?;
	ignore_missing(entity.call_method(vm, "onAfterInit", ()))?;
	Ok(entity)
}

/// Kill entity `idx`: it stops being alive, then its script `onFinish` (which frees its
/// tile and unregisters it from the managers) runs, once. Verified against the original.
fn kill(m: &Rc<RefCell<EntityMachine>>, vm: &SQVM, idx: usize) -> Result<(), String> {
	let entity = {
		let mut machine = m.borrow_mut();
		let rec = &mut machine.entities[idx];
		if !rec.alive {
			return Ok(());
		}
		rec.alive = false;
		rec.entity.clone()
	};
	ignore_missing(entity.call_method(vm, "onFinish", ()))
}

/// Kill entity `idx` if its `fadeAndDie` delay has run out by real time `now` (ms).
/// Returns whether it is dead.
fn reap(m: &Rc<RefCell<EntityMachine>>, vm: &SQVM, idx: usize, now: i64) -> Result<bool, String> {
	let due = {
		let machine = m.borrow();
		let rec = &machine.entities[idx];
		if !rec.alive {
			return Ok(true);
		}
		rec.dies_at.is_some_and(|t| now >= t)
	};
	if due {
		kill(m, vm, idx)?;
	}
	Ok(due)
}

/// The `X`/`Y` of a script vector.
fn vec_xy(vm: &SQVM, v: &SqStrongRef) -> (f64, f64) {
	let x = v.get_member::<Number>(vm, "X").map_or(0.0, |n| n.0);
	let y = v.get_member::<Number>(vm, "Y").map_or(0.0, |n| n.0);
	(x, y)
}

/// The axial tile under a world pixel position (`World.worldToTile`).
fn tile_under(vm: &SQVM, pos: &SqStrongRef) -> Result<(i64, i64), String> {
	let world = SqTable::root(vm)
		.get::<SqStrongRef>("World")
		.ok_or("World table missing")?;
	let coords = world
		.call_method_ret::<SqStrongRef, _>(vm, "worldToTile", (pos.clone(),))?
		.ok_or("worldToTile returned null")?;
	let q = coords.get_member::<i64>(vm, "X").ok_or("worldToTile: no X")?;
	let r = coords.get_member::<i64>(vm, "Y").ok_or("worldToTile: no Y")?;
	Ok((q, r))
}

/// A script array of the entities at `hits` (indices into `refs`).
/// The hits of a spatial query (indices into `records`) as a script array.
fn entity_array(vm: &SQVM, records: &[EntityRecord], hits: Vec<usize>) -> SqStrongRef {
	let mut arr = SqArray::new(vm);
	for i in hits {
		arr.push(records[i].entity.clone());
	}
	arr.to_strong()
}

impl QueryEntity for EntityRecord {
	fn id(&self) -> i64 {
		self.id
	}
	fn pos(&self) -> (f64, f64) {
		self.pos
	}
	fn rendered_top(&self) -> bool {
		self.rendered_top
	}
	fn tile(&self) -> (i64, i64) {
		(self.q, self.r)
	}
	fn visibility(&self) -> f32 {
		self.visibility
	}
	fn alive(&self) -> bool {
		self.alive
	}
}

/// `World.getTile(q, r).IsOccupied = occupied`.
fn set_tile_occupied(vm: &SQVM, q: i64, r: i64, occupied: bool) -> Result<(), String> {
	let world = SqTable::root(vm)
		.get::<SqStrongRef>("World")
		.ok_or("World table missing")?;
	let tile = world
		.call_method_ret::<SqStrongRef, _>(vm, "getTile", (q, r))?
		.ok_or("spawnLocation: no tile at the entity's coords")?;
	tile.set_member(vm, "IsOccupied", occupied)
}

/// Build the sprite class returned by `addSprite`. Brushes resolve through `sprites`
/// (see [`Sprites::resolve`]).
fn build_sprite_class(vm: &mut SQVM, sprites: &Sprites) -> SqClass<SpriteData> {
	let (set_with, get_with) = (sprites.clone(), sprites.clone());
	ClassBuilder::<SpriteData>::new("WorldEntitySprite")
		// `Alpha` is the colour's top byte, an integer (`0x504b40` family).
		.property(
			"Alpha",
			|s: &SpriteData| s.color.alpha(),
			|s: &mut SpriteData, v: i64| s.color.set_alpha(v),
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
			|s: &SpriteData| s.color.is_fading_done(),
			|_s: &mut SpriteData, _v: bool| {},
		)
		.property_obj_rw(
			"Color",
			|s: &SpriteData, vm: &SQVM| super::color::color_object(vm, s.color.color),
			|s: &mut SpriteData, vm: &SQVM, v: SqStrongRef| {
				s.color.color = super::color::color_argb(vm, &v).ok_or("Color: not a color")?;
				Ok(())
			},
		)
		.property("HasBrush", |s: &SpriteData| !s.brush.is_empty(), |_s: &mut SpriteData, _v: bool| {})
		.method("setBrush", move |s: &mut SpriteData, brush: &str| s.brush = set_with.resolve(brush))
		.method("resetBrush", |s: &mut SpriteData| s.brush.clear())
		.method("getBrush", move |s: &mut SpriteData, vm: &SQVM| -> Option<SqStrongRef> {
			get_with.brush_object(vm, &s.brush)
		})
		.method("setOffset", |s: &mut SpriteData, vm: &SQVM, offset: Retained| {
			s.offset = vec_xy(vm, &offset.0)
		})
		.method("setHorizontalFlipping", |s: &mut SpriteData, flipped: bool| s.flipped = flipped)
		// Fades run on real time and finish when the sprite is drawn (see `SpriteColor`).
		.method("fadeIn", |s: &mut SpriteData, vm: &SQVM, ms: i64| s.color.fade_in(ms, real_time(vm)))
		.method("fadeOut", |s: &mut SpriteData, vm: &SQVM, ms: i64| {
			s.color.fade_out(ms, real_time(vm), false)
		})
		.method("fadeOutAndHide", |s: &mut SpriteData, vm: &SQVM, ms: i64| {
			s.color.fade_out(ms, real_time(vm), true)
		})
		.method("fadeToAlpha", |s: &mut SpriteData, vm: &SQVM, alpha: i64, ms: i64| {
			s.color.fade_to_alpha(alpha, ms, real_time(vm))
		})
		.build(vm)
}

/// Build the inert label stub class returned by `addLabel`.
fn build_label_class(vm: &mut SQVM) -> SqClass<LabelData> {
	ClassBuilder::<LabelData>::new("WorldEntityLabel")
		.property(
			"Text",
			|l: &LabelData| SqBytes(l.text.clone()),
			|l: &mut LabelData, v: SqBytes| l.text = v.0,
		)
		.property(
			"Visible",
			|l: &LabelData| l.visible,
			|l: &mut LabelData, v: bool| l.visible = v,
		)
		.property_obj_rw(
			"Color",
			|l: &LabelData, vm: &SQVM| super::color::color_object(vm, l.color),
			|l: &mut LabelData, vm: &SQVM, v: SqStrongRef| {
				l.color = super::color::color_argb(vm, &v).ok_or("Color: not a color")?;
				Ok(())
			},
		)
		.property_obj_rw(
			"Offset",
			|l: &LabelData, vm: &SQVM| {
				let create = SqTable::root(vm).get::<SqStrongRef>("createVec")?;
				create.call_ret::<SqStrongRef, _>(vm, l.offset).ok().flatten()
			},
			|l: &mut LabelData, vm: &SQVM, v: SqStrongRef| {
				l.offset = vec_xy(vm, &v);
				Ok(())
			},
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
				ids: EntityIds::default(),
				entities: Vec::new(),
				by_identity: HashMap::new(),
				by_id: HashMap::new(),
				methods: Vec::new(),
				player_view: view::PlayerView::default(),
				footprints: footprints::Footprints::default(),
				thinkers: think::Thinkers::default(),
				sprites: Sprites::default(),
			})),
		}
	}

	/// Use `sprites` to resolve sprite brushes. Call before [`install`](Self::install).
	/// The entity id counter, to share with the battle's units.
	pub fn ids(&self) -> EntityIds {
		self.machine.borrow().ids.clone()
	}

	pub fn set_sprites(&self, sprites: Sprites) {
		self.machine.borrow_mut().sprites = sprites;
	}

	/// Build the stub classes + the shared entity methods, and register
	/// `World.spawnLocation` (2-arg `coords` and 3-arg `x,y` overloads). Call once with
	/// `&mut SQVM`, after the `World` table + grid natives exist.
	pub fn install(&self, vm: &mut SQVM) {
		let sprites = self.machine.borrow().sprites.clone();
		self.machine.borrow_mut().sprite_class = Some(build_sprite_class(vm, &sprites));
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
		// setPos(vec): move to a world pixel position; the entity's tile follows.
		let set_pos = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM, pos: Retained| -> Result<(), String> {
				let xy = vec_xy(vm, &pos.0);
				let (q, r) = tile_under(vm, &pos.0)?;
				if let Some(rec) = m.borrow_mut().record_mut(vm.this_identity()) {
					rec.pos = xy;
					(rec.q, rec.r) = (q, r);
				}
				Ok(())
			})
		};
		// getRelativePos: the position as stored (0x49c710), which for a world entity is
		// its world position, as getPos.
		let get_relative_pos = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM| -> Result<Option<SqStrongRef>, String> {
				let (x, y) = m.borrow().record(vm.this_identity()).map_or((0.0, 0.0), |rec| rec.pos);
				let create_vec = SqTable::root(vm)
					.get::<SqStrongRef>("createVec")
					.ok_or("getRelativePos: createVec missing")?;
				create_vec.call_ret::<SqStrongRef, _>(vm, (x, y))
			})
		};
		// getPos: the entity's world pixel position.
		let get_pos = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM| -> Result<Option<SqStrongRef>, String> {
				let (x, y) = m
					.borrow()
					.record(vm.this_identity())
					.map(|rec| rec.pos)
					.ok_or("getPos: entity not registered")?;
				let create_vec = SqTable::root(vm)
					.get::<SqStrongRef>("createVec")
					.ok_or("getPos: createVec missing")?;
				create_vec.call_ret::<SqStrongRef, _>(vm, (x, y))
			})
		};
		// addSprite(name): a new sprite on top of the others; a name already there gets
		// that sprite back (the original logs that it already exists, 0x4a7dc0).
		let add_sprite = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM, name: String| -> Option<SqStrongRef> {
				sprites::add(&m, vm, name, false)
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
		let remove_label = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM, name: String| {
				if let Some(rec) = m.borrow_mut().record_mut(vm.this_identity()) {
					rec.labels.remove(&name);
				}
			})
		};
		let remove_all_labels = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM| {
				if let Some(rec) = m.borrow_mut().record_mut(vm.this_identity()) {
					rec.labels.clear();
				}
			})
		};
		// Render/sim no-ops: accept any args, do nothing (shared across all the names).
		let noop = vm.make_closure(|_a: Variadic<Retained>| {});

		let mut methods: Vec<(&'static str, SqStrongRef)> = vec![
			("getID", get_id),
			("getTile", get_tile),
			("getPos", get_pos),
			("getRelativePos", get_relative_pos),
			("setPos", set_pos),
			("addSprite", add_sprite),
			("getSprite", get_sprite),
			("hasSprite", has_sprite),
			("addLabel", add_label),
			("getLabel", get_label),
			("hasLabel", has_label),
			("getFaction", get_faction),
			("setFactionEx", set_faction_ex),
		];
		// die / fadeAndDie: run the entity's script `onFinish` (which marks it dead,
		// frees its tile and unregisters it from the managers) once; later calls are
		// no-ops. Verified against the original (2026-10-13). Rendering has no fade.
		let die = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM| -> Result<(), String> {
				let idx = m.borrow().by_identity.get(&vm.this_identity()).copied();
				idx.map_or(Ok(()), |idx| kill(&m, vm, idx))
			})
		};
		// fadeAndDie(): the original fades the entity out and lets it die 500 ms of real
		// time later (from the next think or render pass after that); until then it is
		// still alive and found by queries. Later calls don't extend the delay.
		let fade_and_die = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM| -> Result<(), String> {
				let now = render::real_time_ms(vm)?;
				if let Some(rec) = m.borrow_mut().record_mut(vm.this_identity()) {
					if rec.alive && rec.dies_at.is_none() {
						rec.dies_at = Some(now + 500);
					}
				}
				Ok(())
			})
		};
		methods.extend(view::entity_methods(&self.machine, vm));
		methods.extend(think::entity_methods(&self.machine, vm));
		methods.extend(movement::entity_methods(&self.machine, vm));
		methods.extend(sprites::entity_methods(&self.machine, vm));
		methods.push(("removeLabel", remove_label));
		methods.push(("removeAllLabels", remove_all_labels));
		methods.push(("isInitialized", vm.make_closure(|| true)));
		methods.push(("isStoringColor", vm.make_closure(|| false)));
		methods.push(("die", die));
		methods.push(("fadeAndDie", fade_and_die));
		// Render-only natives.
		for name in [
			"setZ",
			"storeSpriteColors",
			"restoreSpriteColors",
			"fadeTo",
			"fadeToStoredColors",
			"fadeIn",
			"fadeOut",
			"setAlpha",
			"setSpriteOcclusion",
			"moveSpriteOffset",
			"moveSpriteScale",
			"setRenderCallbackEnabled",
		] {
			methods.push((name, noop.clone()));
		}
		self.machine.borrow_mut().methods = methods;

		// --- World.spawnLocation / World.spawnEntity: Sqrat overloads (1 ctx / 2 coords /
		// 3 x,y). We serve the 2- and 3-arg forms; the 1-arg context form errors (the
		// scripts never use it). Locations occupy their tile, parties don't.
		let mut slots: Vec<(String, SqStrongRef)> = Vec::new();
		for (name, occupy) in [("spawnLocation", true), ("spawnEntity", false)] {
			let spawn_2 = {
				let m = self.machine.clone();
				vm.make_closure(
					move |vm: &SQVM, path: &str, coords: SqTable| -> Result<SqStrongRef, String> {
						let q = coords.get::<i64>("X").ok_or("spawn: coords table needs X")?;
						let r = coords.get::<i64>("Y").ok_or("spawn: coords table needs Y")?;
						spawn(&m, vm, path, q, r, occupy)
					},
				)
			};
			let spawn_3 = {
				let m = self.machine.clone();
				vm.make_closure(
					move |vm: &SQVM, path: &str, x: i64, y: i64| -> Result<SqStrongRef, String> {
						spawn(&m, vm, path, x, y, occupy)
					},
				)
			};
			let dispatch = make_dispatcher(vm, name);
			slots.push((overload_key(name, 2), spawn_2));
			slots.push((overload_key(name, 3), spawn_3));
			slots.push((name.to_string(), dispatch));
		}
		// World spatial queries and id lookup over the live entities.
		let all_at_pos = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM, pos: Retained, radius: Number| -> SqStrongRef {
				let center = vec_xy(vm, &pos.0);
				let machine = m.borrow();
				entity_array(vm, &machine.entities, world_query::all_at_pos(center, radius.0, &machine.entities))
			})
		};
		let all_and_one_location_at_pos = {
			let m = self.machine.clone();
			vm.make_closure(
				move |vm: &SQVM, pos: Retained, radius: Number| -> Result<SqStrongRef, String> {
					let center = vec_xy(vm, &pos.0);
					let tile = tile_under(vm, &pos.0)?;
					let machine = m.borrow();
					let hits = world_query::all_and_one_location_at_pos(center, radius.0, tile, &machine.entities);
					Ok(entity_array(vm, &machine.entities, hits))
				},
			)
		};
		let all_visible_at_pos = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM, pos: Retained, radius: Number| -> SqStrongRef {
				let center = vec_xy(vm, &pos.0);
				let machine = m.borrow();
				let hits = world_query::all_visible_at_pos(center, radius.0, &machine.entities);
				entity_array(vm, &machine.entities, hits)
			})
		};
		// getPlayerEntity(): the first entity by id in the player's faction (from the
		// original's disassembly).
		let get_player_entity = {
			let m = self.machine.clone();
			vm.make_closure(move || -> Option<SqStrongRef> {
				let machine = m.borrow();
				machine
					.entities
					.iter()
					.filter(|rec| rec.alive && rec.faction == 1)
					.min_by_key(|rec| rec.id)
					.map(|rec| rec.entity.clone())
			})
		};
		let get_entity_at_tile = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM, coords: Retained| -> Option<SqStrongRef> {
				let q = coords.0.get_member::<i64>(vm, "X")?;
				let r = coords.0.get_member::<i64>(vm, "Y")?;
				let machine = m.borrow();
				world_query::entity_at_tile((q, r), &machine.entities).map(|i| machine.entities[i].entity.clone())
			})
		};
		let get_entity_by_id = {
			let m = self.machine.clone();
			vm.make_closure(move |id: i64| -> Option<SqStrongRef> {
				let machine = m.borrow();
				let rec = &machine.entities[*machine.by_id.get(&id)?];
				rec.alive.then(|| rec.entity.clone())
			})
		};
		slots.push(("getAllEntitiesAtPos".to_string(), all_at_pos));
		slots.push(("getAllEntitiesAndOneLocationAtPos".to_string(), all_and_one_location_at_pos));
		slots.push(("getEntityByID".to_string(), get_entity_by_id));
		slots.push(("getEntityAtTile".to_string(), get_entity_at_tile));
		slots.push(("getPlayerEntity".to_string(), get_player_entity));
		slots.push(("getAllEntitiesVisibleAtPos".to_string(), all_visible_at_pos));
		slots.extend(view::world_methods(&self.machine, vm));
		slots.extend(render::world_methods(&self.machine, vm));
		slots.extend(footprints::world_methods(&self.machine, vm));

		let mut world = SqTable::root(vm)
			.get::<SqTable>("World")
			.expect("World table must be installed before WorldEntities");
		for (key, closure) in slots {
			world.set(&key, closure);
		}
	}

	/// The per-frame think pass (see [`think`]), for the world scene's update. The
	/// returned hook shares this host's state.
	pub fn scene_update_hook(&self) -> impl Fn(&SQVM) -> Result<(), String> + 'static {
		let m = self.machine.clone();
		move |vm: &SQVM| think::run_pass(&m, vm)
	}

	/// The world entities' share of a save (see [`persist`]).
	pub(crate) fn save_part(&self) -> Rc<dyn crate::native::save::SavePart> {
		Rc::new(persist::EntitiesPart(self.machine.clone()))
	}

	/// The per-frame render pass (see [`render`]): which entities the player discovers.
	pub fn scene_render_hook(&self) -> impl Fn(&SQVM) -> Result<(), String> + 'static {
		let m = self.machine.clone();
		move |vm: &SQVM| render::run_pass(&m, vm)
	}

	/// Number of spawned entities (host/test inspection).
	pub fn count(&self) -> usize {
		self.machine.borrow().entities.len()
	}

	/// The footprints drawn now: none unless `ShowTracking` is on (`0x4e49a0`).
	pub(crate) fn scene_footprints(&self, vm: &SQVM) -> Vec<crate::scene::SceneFootprint> {
		let tracking = SqTable::root(vm)
			.get::<SqStrongRef>("Settings")
			.and_then(|s| s.call_method_ret::<SqStrongRef, _>(vm, "getTempGameplaySettings", ()).ok().flatten())
			.and_then(|t| t.get_member::<bool>(vm, "ShowTracking"))
			.unwrap_or(true);
		if !tracking {
			return Vec::new();
		}
		let now = footprints::virtual_time(vm);
		let mut m = self.machine.borrow_mut();
		m.footprints.expire(now);
		let view = &m.player_view;
		let (pos, vision) = ((view.pos.0 as f32, view.pos.1 as f32), view.vision_radius);
		m.footprints
			.drawn(now, pos, vision)
			.into_iter()
			.map(|(f, alpha)| crate::scene::SceneFootprint {
				brush: f.brush,
				pos: f.pos,
				scale: f.scale,
				alpha,
			})
			.collect()
	}

	/// Every live entity and its sprites as the renderer sees them.
	/// What entity `id`'s portrait draws (`None` for no such live entity).
	pub(crate) fn portrait(&self, vm: &SQVM, id: i64) -> Option<crate::scene::ScenePortrait> {
		let m = self.machine.borrow();
		let class = m.sprite_class.clone()?;
		let rec = m.entities.iter().find(|rec| rec.alive && rec.id == id)?;
		let sprites = rec
			.sprites
			.iter()
			.filter_map(|(name, handle)| {
				class.with_payload(vm, handle, |s| crate::scene::PortraitSprite {
					name: name.clone(),
					brush: s.brush.clone(),
					color: s.color.color,
					scale: s.scale as f32,
					rotation: 0.0,
					saturation: 1.0,
					flipped: s.flipped,
					offset: (s.offset.0 as f32, s.offset.1 as f32),
					visible: s.visible,
					in_portrait: s.in_portrait,
				})
			})
			.collect();
		Some(crate::scene::ScenePortrait {
			sprites,
			apply_offsets: rec.portrait_offsets,
		})
	}

	pub(crate) fn scene_entities(&self, vm: &SQVM) -> Vec<crate::scene::SceneEntity> {
		let now = real_time(vm);
		let m = self.machine.borrow();
		let (Some(class), Some(label_class)) = (m.sprite_class.clone(), m.label_class.clone()) else {
			return Vec::new();
		};
		m.entities
			.iter()
			.filter(|rec| rec.alive)
			.map(|rec| crate::scene::SceneEntity {
				id: rec.id,
				pos: rec.pos,
				shown: rec.render.shown(),
				sprites: rec
					.sprites
					.iter()
					.filter_map(|(name, handle)| {
						class.with_payload(vm, handle, |s| crate::scene::SceneSprite {
							name: name.clone(),
							brush: s.brush.clone(),
							color: s.color.shown(now),
							scale: s.scale,
							visible: s.visible,
							offset: s.offset,
							flipped: s.flipped,
							ignore_ambient: s.ignore_ambient,
							scaling: s.scaling,
							colorization: s.colorization,
						})
					})
					.collect(),
				labels: rec
					.labels
					.iter()
					.filter_map(|(name, handle)| {
						label_class.with_payload(vm, handle, |l| crate::scene::SceneLabel {
							name: name.clone(),
							text: l.text.clone(),
							visible: l.visible,
							color: l.color,
							offset: l.offset,
						})
					})
					.collect(),
			})
			.collect()
	}

	/// Release every retained ref (instances, their sprites/labels, the stub classes,
	/// the method closures). Call before the VM is closed.
	pub fn clear(&self) {
		let mut m = self.machine.borrow_mut();
		m.entities.clear();
		m.by_identity.clear();
		m.by_id.clear();
		m.thinkers = think::Thinkers::default();
		m.methods.clear();
		m.sprite_class = None;
		m.label_class = None;
	}
}
