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

use squirrel::{ClassBuilder, Number, Retained, SQVM, SqArray, SqBytes, SqClass, SqStrongRef, SqTable};

use super::inject::inject_natives;
use super::overload::{make_dispatcher, overload_key, single_overload};
use super::world_layout;
use super::world_query::{self, QueryEntity};
use crate::module::Instantiator;
use crate::native::{EntityIds, Sprites};
use crate::native::spin::Spin;
use crate::native::sprite_anim::Anim;
use crate::native::sprite_class::{self, Flag, Kind, Num, SpriteHandle, SpriteState};
use crate::native::sprite_color::{SpriteColor, real_time};

mod footprints;
mod hover;
mod movement;
mod original;
pub(crate) use original::{argb, rgba};
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
	/// Degrees, counter-clockwise (`Rotation`).
	rotation: f32,
	saturation: f64,
	/// `Highlight` (ARGB), blended over the sprite by `HighlightFactor`.
	highlight: u32,
	highlight_factor: f64,
	ignore_camera_flip: bool,
	/// Its `Torque` (see `spin`).
	spin: Spin,
	/// The frame of its brush it shows (see `sprite_anim`).
	anim: Anim,
	/// The flag bits its brush calls left (see [`Sprites::brush_bits`]), which the
	/// original saves.
	brush_bits: u32,
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
			rotation: 0.0,
			saturation: 1.0,
			highlight: 0,
			highlight_factor: 0.0,
			ignore_camera_flip: false,
			spin: Spin::default(),
			anim: Anim::default(),
			brush_bits: 0,
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
	/// `BackgroundColor` (ARGB) and `BackgroundVisible`: the box behind its text.
	background_color: u32,
	background_visible: bool,
	/// `HAlign`, `VAlign`, `Size` and `Z`, kept as written: reading one crashes the
	/// original (`HAlign` probed 2026-10-14), so no script does.
	h_align: f64,
	v_align: f64,
	size: f64,
	z: f64,
}

impl Default for LabelData {
	fn default() -> Self {
		LabelData {
			text: Vec::new(),
			visible: false,
			color: 0xffff_ffff,
			offset: (0.0, 0.0),
			// A fresh one's, probed 2026-10-14.
			background_color: 0x6600_0000,
			background_visible: true,
			h_align: 0.0,
			v_align: 0.0,
			size: 0.0,
			z: 0.0,
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
	/// The script object; let go once the entity dies (see [`release`]).
	entity: Option<SqStrongRef>,
	/// After its death, a script weak reference to the object, which goes null once
	/// nothing holds it: the slot is then free for a new entity.
	weak: Option<SqStrongRef>,
	/// Its class levels' identities, its keys in [`EntityMachine::by_identity`].
	identities: Vec<i64>,
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
	/// Entity id → its record index in `entities` (`getEntityByID`); the living only.
	by_id: HashMap<i64, usize>,
	/// The slots of dead entities, oldest first, to take for new ones once their script
	/// objects are gone.
	dead: Vec<usize>,
	/// Passes over the slots running scripts (think, sight): no slot is taken meanwhile.
	walking: u32,
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
		let mut m = machine.borrow_mut();
		m.by_identity.insert(identity, record_idx);
		m.entities[record_idx].identities.push(identity);
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
	let free = free_slot(machine, vm);
	let record_idx = {
		let mut m = machine.borrow_mut();
		let idx = free.unwrap_or(m.entities.len());
		m.by_id.insert(id, idx);
		let record = EntityRecord {
			id,
			path: path.to_string(),
			entity: Some(entity.clone()),
			weak: None,
			identities: Vec::new(),
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
		};
		match free {
			Some(i) => m.entities[i] = record,
			None => m.entities.push(record),
		}
		idx
	};
	inject_levels(machine, vm, &entity, record_idx);
	Ok((entity, record_idx))
}

/// How many dead slots a new entity looks at for one to take.
const SLOTS_LOOKED_AT: usize = 16;

/// A dead entity's slot whose script object is gone, for a new entity, cleared of what
/// pointed at it (its identity keys, thinker entries). None during a pass over the
/// slots (think, sight), which holds them.
fn free_slot(m: &Rc<RefCell<EntityMachine>>, vm: &SQVM) -> Option<usize> {
	let candidates: Vec<(usize, Option<SqStrongRef>)> = {
		let machine = m.borrow();
		if machine.walking > 0 {
			return None;
		}
		machine.dead.iter().take(SLOTS_LOOKED_AT).map(|&i| (i, machine.entities[i].weak.clone())).collect()
	};
	let found = candidates.iter().position(|(_, weak)| weak.as_ref().is_none_or(|w| w.weak_is_gone(vm)));
	let mut machine = m.borrow_mut();
	let Some(pos) = found else {
		// All held still (a contract's reference, say): look at others next time.
		let n = candidates.len().min(machine.dead.len());
		machine.dead.rotate_left(n);
		return None;
	};
	let idx = machine.dead.remove(pos);
	let machine = &mut *machine;
	for identity in std::mem::take(&mut machine.entities[idx].identities) {
		if machine.by_identity.get(&identity) == Some(&idx) {
			machine.by_identity.remove(&identity);
		}
	}
	machine.thinkers.forget(idx);
	Some(idx)
}

/// A dead entity lets go of its script object. Its record stays, sprites and labels
/// too, while scripts may still hold the object and call its natives (which find it
/// dead), with a weak reference that tells when the object is gone and the slot free.
fn release(m: &Rc<RefCell<EntityMachine>>, vm: &SQVM, idx: usize) {
	let mut machine = m.borrow_mut();
	let rec = &mut machine.entities[idx];
	let Some(entity) = rec.entity.take() else { return };
	rec.weak = Some(entity.downgrade(vm));
	let id = rec.id;
	if machine.by_id.get(&id) == Some(&idx) {
		machine.by_id.remove(&id);
	}
	machine.dead.push(idx);
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
	let Some(entity) = entity else { return Ok(()) };
	let finished = ignore_missing(entity.call_method(vm, "onFinish", ()));
	release(m, vm, idx);
	finished
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

impl SpriteState for SpriteData {
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
			Num::Rotation => f64::from(self.rotation),
			Num::Saturation => self.saturation,
			Num::HighlightFactor => self.highlight_factor,
		}
	}
	fn set_num(&mut self, n: Num, v: f64) {
		match n {
			Num::Scale => self.scale = v,
			Num::Rotation => self.rotation = v as f32,
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

impl SpriteHandle for SpriteData {
	fn with<R>(&self, f: impl FnOnce(&dyn SpriteState) -> R) -> R {
		f(self)
	}
	fn with_mut<R>(&mut self, f: impl FnOnce(&mut dyn SpriteState) -> R) -> R {
		f(self)
	}
}

/// Build the sprite class `addSprite` hands out (see `sprite_class`).
fn build_sprite_class(vm: &mut SQVM, sprites: &Sprites) -> SqClass<SpriteData> {
	let b = ClassBuilder::<SpriteData>::new("WorldEntitySprite");
	sprite_class::members(b, Kind { sprites: sprites.clone(), battle: false, moving: None }).build(vm)
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
		.property_obj_rw(
			"BackgroundColor",
			|l: &LabelData, vm: &SQVM| super::color::color_object(vm, l.background_color),
			|l: &mut LabelData, vm: &SQVM, v: SqStrongRef| {
				l.background_color = super::color::color_argb(vm, &v).ok_or("BackgroundColor: not a color")?;
				Ok(())
			},
		)
		.property("BackgroundVisible", |l: &LabelData| l.background_visible, |l: &mut LabelData, v: bool| {
			l.background_visible = v
		})
		.property("HAlign", |l: &LabelData| l.h_align, |l: &mut LabelData, v: f64| l.h_align = v)
		.property("VAlign", |l: &LabelData| l.v_align, |l: &mut LabelData, v: f64| l.v_align = v)
		.property("Size", |l: &LabelData| l.size, |l: &mut LabelData, v: f64| l.size = v)
		.property("Z", |l: &LabelData| l.z, |l: &mut LabelData, v: f64| l.z = v)
		.build(vm)
}

/// The native World-entity subsystem (`World.spawnLocation` + injected entity methods).
#[derive(Clone)]
pub struct WorldEntities {
	machine: Rc<RefCell<EntityMachine>>,
	hover: hover::Hover,
}

impl WorldEntities {
	/// Create the subsystem. `instantiate` is the `this.new` capability (from
	/// [`ModuleSystem::instantiator`](crate::ModuleSystem::instantiator)).
	pub fn new(instantiate: Instantiator) -> Self {
		WorldEntities {
			hover: hover::Hover::default(),
			machine: Rc::new(RefCell::new(EntityMachine {
				instantiate,
				sprite_class: None,
				label_class: None,
				ids: EntityIds::default(),
				entities: Vec::new(),
				by_identity: HashMap::new(),
				by_id: HashMap::new(),
				dead: Vec::new(),
				walking: 0,
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
				// The original's label is a scene entity (a child `Text`), which takes an id.
				m.borrow().ids.next();
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
		// fadeAndDie(): the original fades the entity's sprites out over 500 ms and lets
		// it die 500 ms of real time later (from the next think or render pass after
		// that; `0x4e90c0`); until then it is still alive and found by queries. Later
		// calls don't extend the delay.
		let fade_and_die = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM| -> Result<(), String> {
				let now = render::real_time_ms(vm)?;
				let idx = {
					let mut machine = m.borrow_mut();
					let idx = machine.by_identity.get(&vm.this_identity()).copied();
					let rec = idx.and_then(|i| machine.entities.get_mut(i)).filter(|r| r.alive && r.dies_at.is_none());
					rec.map(|r| r.dies_at = Some(now + 500)).and(idx)
				};
				if let Some(idx) = idx {
					render::fade_sprites(&m, vm, idx, render::Fade::Out(500), now as f32 * 0.001);
				}
				Ok(())
			})
		};
		// fadeIn(ms), fadeOut(ms), setAlpha(alpha): to every sprite (`0x4a9930`,
		// `0x4a99c0`, `0x4a9ac0`), on real time.
		let entity_fade = |make: fn(i64) -> render::Fade| {
			let m = self.machine.clone();
			move |vm: &SQVM, value: Number| -> Result<(), String> {
				let idx = m.borrow().by_identity.get(&vm.this_identity()).copied();
				if let Some(idx) = idx {
					let now = render::real_time_ms(vm)? as f32 * 0.001;
					render::fade_sprites(&m, vm, idx, make(value.0 as i64), now);
				}
				Ok(())
			}
		};
		let fade_in = vm.make_closure(entity_fade(render::Fade::In));
		let fade_out = vm.make_closure(entity_fade(render::Fade::Out));
		let set_alpha = vm.make_closure(entity_fade(render::Fade::Alpha));
		methods.extend(view::entity_methods(&self.machine, vm));
		methods.extend(think::entity_methods(&self.machine, vm));
		methods.extend(movement::entity_methods(&self.machine, vm));
		methods.extend(sprites::entity_methods(&self.machine, vm));
		methods.push(("removeLabel", remove_label));
		methods.push(("removeAllLabels", remove_all_labels));
		methods.push(("isInitialized", vm.make_closure(|| true)));
		methods.push(("isStoringColor", vm.make_closure(|| {
			crate::native::stubs::note("World entity.isStoringColor");
			false
		})));
		methods.push(("die", die));
		methods.push(("fadeAndDie", fade_and_die));
		methods.push(("fadeIn", fade_in));
		methods.push(("fadeOut", fade_out));
		methods.push(("setAlpha", set_alpha));
		// Render-only natives.
		for name in [
			"setZ",
			"fadeTo",
			"fadeToStoredColors",
			"setSpriteOcclusion",
			"moveSpriteOffset",
			"moveSpriteScale",
			"setRenderCallbackEnabled",
		] {
			methods.push((name, crate::native::stubs::closure(vm, crate::native::stubs::name(format!("World entity.{name}")))));
		}
		// One-arity Sqrat overloads in the original; render-only here.
		for (name, key) in [
			("storeSpriteColors", "__sqrat_ol_ storeSpriteColors_0"),
			("restoreSpriteColors", "__sqrat_ol_ restoreSpriteColors_0"),
		] {
			let leaf = crate::native::stubs::closure(vm, crate::native::stubs::name(format!("World entity.{name}")));
			methods.extend(single_overload(vm, name, key, leaf));
		}
		self.machine.borrow_mut().methods = methods;

		// --- World.spawnLocation / World.spawnEntity: Sqrat overloads (script, at the tile
		// under the mouse (see `hover`) / script, coords / script, x, y). Locations occupy
		// their tile, parties don't.
		let mut slots: Vec<(String, SqStrongRef)> = Vec::new();
		for (name, occupy) in [("spawnLocation", true), ("spawnEntity", false)] {
			let spawn_1 = {
				let m = self.machine.clone();
				let hover = self.hover.clone();
				vm.make_closure(move |vm: &SQVM, path: &str| -> Result<SqStrongRef, String> {
					// The original reads the tile without a check (a crash before the
					// pointer has been over the map).
					let (q, r) = hover.tile().ok_or("spawn: the mouse hasn't been over the map")?;
					spawn(&m, vm, path, q, r, occupy)
				})
			};
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
			slots.push((overload_key(name, 1), spawn_1));
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
					.and_then(|rec| rec.entity.clone())
			})
		};
		let get_entity_at_tile = {
			let m = self.machine.clone();
			vm.make_closure(move |vm: &SQVM, coords: Retained| -> Option<SqStrongRef> {
				let q = coords.0.get_member::<i64>(vm, "X")?;
				let r = coords.0.get_member::<i64>(vm, "Y")?;
				let machine = m.borrow();
				world_query::entity_at_tile((q, r), &machine.entities).and_then(|i| machine.entities[i].entity.clone())
			})
		};
		let get_entity_by_id = {
			let m = self.machine.clone();
			vm.make_closure(move |id: i64| -> Option<SqStrongRef> {
				let machine = m.borrow();
				let rec = &machine.entities[*machine.by_id.get(&id)?];
				rec.alive.then(|| rec.entity.clone()).flatten()
			})
		};
		slots.push(("getAllEntitiesAtPos".to_string(), all_at_pos));
		// getEntityAtPos(pos) or (x, y): Sqrat overloads over the entity layer's point
		// query (radius 4, `0x4f2ae0`/`0x4f2c20`). That query (`0x55cb60`) only matches an
		// entity when an edge of its rect lies within 4 of the point on each axis, and
		// no point near the company ever matched (probed 2026-10-14: 2 x 11,817 points
		// around it, at campaign start and 1.5 s later). Nothing in the game or the
		// installed mods calls it. Always null here.
		slots.push((overload_key("getEntityAtPos", 1), vm.make_closure(|_pos: Retained| -> Option<SqStrongRef> { None })));
		slots.push((
			overload_key("getEntityAtPos", 2),
			vm.make_closure(|_x: Number, _y: Number| -> Option<SqStrongRef> { None }),
		));
		slots.push(("getEntityAtPos".to_string(), make_dispatcher(vm, "getEntityAtPos")));
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
	/// The world's own mouse listener (see [`hover`]): keeps the tile under the pointer,
	/// read through `camera` on a map of the size `dims` gives.
	pub(crate) fn mouse_listener(
		&self,
		camera: crate::native::world_camera::WorldCamera,
		dims: impl Fn() -> (i64, i64) + 'static,
	) -> impl Fn(&SQVM, &crate::input_events::MouseInput) -> Result<(), String> + 'static {
		let hover = self.hover.clone();
		move |vm, input| {
			hover.on_mouse(vm, &camera, dims(), input);
			Ok(())
		}
	}

	pub fn scene_render_hook(&self) -> impl Fn(&SQVM) -> Result<(), String> + 'static {
		let m = self.machine.clone();
		move |vm: &SQVM| render::run_pass(&m, vm)
	}

	/// Number of entity slots, the living and the dead not yet taken over (host/test
	/// inspection).
	pub fn count(&self) -> usize {
		self.machine.borrow().entities.len()
	}

	/// Number of registered thinkers (host/test inspection).
	pub fn thinker_count(&self) -> usize {
		self.machine.borrow().thinkers.len()
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

	/// Turn the live entities' sprites that have a torque to virtual time `now`, in
	/// seconds (see `spin`), and step their animations to virtual millisecond `now_ms`
	/// (see `sprite_anim`).
	pub(crate) fn turn(&self, vm: &SQVM, now: f32, now_ms: u32) {
		let m = self.machine.borrow();
		let Some(class) = m.sprite_class.clone() else { return };
		for rec in m.entities.iter().filter(|rec| rec.alive) {
			for (_, handle) in rec.sprites.iter() {
				class.with_payload(vm, handle, |s| {
					s.rotation = s.spin.turn(s.rotation, now);
					if let Some(scale) = s.anim.step_on(&m.sprites, &s.brush, now_ms) {
						s.scale = f64::from(scale);
					}
				});
			}
		}
	}

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
					rotation: s.rotation,
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
		// By id: a new entity may take a dead one's slot.
		let mut live: Vec<&EntityRecord> = m.entities.iter().filter(|rec| rec.alive).collect();
		live.sort_by_key(|rec| rec.id);
		live.into_iter()
			.map(|rec| crate::scene::SceneEntity {
				id: rec.id,
				pos: rec.pos,
				// In sight, or still fading out of it (see `render`): a shown sprite's fade
				// (a hidden one's never finishes, as it isn't drawn).
				shown: rec.render.shown()
					|| rec
						.sprites
						.iter()
						.any(|(_, h)| class.with_payload(vm, h, |s| s.visible && !s.color.is_fading_done()).unwrap_or(false)),
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
							rotation: s.rotation,
							ignore_ambient: s.ignore_ambient,
							scaling: s.scaling,
							colorization: s.colorization,
							frame: s.anim.frame(),
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
							// Labels don't fade: they show while the entity is in sight.
							visible: l.visible && rec.render.shown(),
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
		m.dead.clear();
		m.thinkers = think::Thinkers::default();
		m.methods.clear();
		m.sprite_class = None;
		m.label_class = None;
	}
}
