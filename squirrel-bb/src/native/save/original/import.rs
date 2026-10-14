//! Loading a campaign save of the original's layout, as its `World.load` (`0x4f4ac0`)
//! does (see [`super`] for the order).

use squirrel::{SQVM, SqStrongRef, SqTable};

use super::super::LoadCx;
use super::camera::{Camera, Clock};
use super::fog;
use super::record::{Body, Head, TacticalNative};
use super::scene::Scene;
use super::world::{OriginalWorld, Roster};

/// The script file of class hash `hash` (`IO.scriptFilenameByHash`, as the original's
/// load looks it up).
fn script_path(vm: &SQVM, hash: i32) -> Result<String, String> {
	let io = SqTable::root(vm).get::<SqStrongRef>("IO").ok_or("IO missing")?;
	io.call_method_ret::<String, _>(vm, "scriptFilenameByHash", (i64::from(hash),))?
		.ok_or_else(|| format!("the save names a script that doesn't exist (class hash {hash})"))
}

/// The world entities and footprints of the scene's layers, in the layers' order.
fn load_entities(world: &dyn OriginalWorld, vm: &SQVM, scene: &Scene) -> Result<(), String> {
	for layer in &scene.layers {
		let top = layer.name == b"entity";
		for e in layer.entities() {
			match &e.body {
				Body::World(part) => {
					let path = script_path(vm, part.script_hash)?;
					world.load_entity(vm, &path, top, &e.head, part)?;
				}
				Body::Footprint { sprite, time, kind } => world.load_footprint(&e.head, sprite, *time, *kind),
				// Decorations and region names are the map's (`load_map`).
				_ => {}
			}
		}
	}
	Ok(())
}

/// A roster's units: a `u16` count, then each unit's record, its native part, and its
/// script's data.
fn load_roster(world: &dyn OriginalWorld, cx: &LoadCx, roster: Roster, count: u16) -> Result<(), String> {
	for _ in 0..count {
		let (head, native) = {
			let mut r = cx.reader.borrow_mut();
			(Head::read(&mut r)?, TacticalNative::read(&mut r)?)
		};
		let path = script_path(cx.vm, native.script_hash)?;
		let unit = world.load_unit(cx.vm, &path, &head, &native)?;
		unit.call_method(cx.vm, "onDeserialize", (cx.stream.clone(),))?;
		world.add_unit(roster, unit);
	}
	Ok(())
}

/// Load the save `cx` reads (its header already read; `version` is its metadata's),
/// with the world state's `onBeforeLoad` and `onLoad` callbacks.
pub(crate) fn load(
	world: &dyn OriginalWorld,
	cx: &LoadCx,
	version: i64,
	before: Option<SqStrongRef>,
	after: Option<SqStrongRef>,
) -> Result<(), String> {
	let vm = cx.vm;
	world.clear(vm)?;
	if let Some(before) = before {
		before.call(vm, (cx.stream.clone(),))?;
	}
	let (vtime, scene) = {
		let mut r = cx.reader.borrow_mut();
		let vtime = if version <= 49 { f64::from(r.f32()?) } else { r.f64()? };
		(vtime, Scene::read(&mut r)?)
	};
	world.set_virtual_time(vtime);
	world.load_map(vm, &scene)?;
	load_entities(world, vm, &scene)?;
	drop(scene);
	let (camera, clock, players) = {
		let mut r = cx.reader.borrow_mut();
		(Camera::read(&mut r)?, Clock::read(&mut r)?, r.u16()?)
	};
	world.load_camera(&camera);
	world.load_clock(&clock);
	load_roster(world, cx, Roster::Player, players)?;
	let guests = cx.reader.borrow_mut().u16()?;
	load_roster(world, cx, Roster::Guests, guests)?;
	let rosters = cx.reader.borrow_mut().u16()?;
	for _ in 0..rosters {
		let (id, count) = {
			let mut r = cx.reader.borrow_mut();
			(r.u32()?, r.u16()?)
		};
		load_roster(world, cx, Roster::Created(id), count)?;
	}
	// The world entities' scripts, by id descending, as they were saved. Each saved id
	// names the entity in that place, or another one by id; with no such entity the
	// original shows a message and reads on misaligned, where this fails the load.
	let scripts = world.entity_scripts();
	for (id, script) in &scripts {
		let saved = cx.reader.borrow_mut().u32()?;
		let target = if saved == *id {
			script
		} else {
			&scripts.iter().find(|(i, _)| *i == saved).ok_or_else(|| {
				format!("A critical error occured: ID mismatch while deserializing script data (entity {id}, the save has {saved})")
			})?
			.1
		};
		target.call_method(vm, "onDeserialize", (cx.stream.clone(),))?;
	}
	if let Some(after) = after {
		after.call(vm, (cx.stream.clone(),))?;
	}
	let mask = fog::read(&mut cx.reader.borrow_mut(), version)?;
	world.load_fog(mask)
}
