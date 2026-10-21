//! Writing a campaign save in the original's layout, as its `World.save` (`0x4f4650`)
//! does (see [`super`] for the order), from the engine's state.


use super::super::{SaveCallback, SaveCx};
use super::fog;
use super::world::{OriginalWorld, SavedUnit};

/// A roster's units: a `u16` count, then each unit's record, native part, and script
/// data.
fn write_units(cx: &SaveCx, units: &[SavedUnit]) -> Result<(), String> {
	cx.writer.borrow_mut().u16(units.len() as u16);
	for u in units {
		{
			let mut w = cx.writer.borrow_mut();
			u.head.write(&mut w);
			u.native.write(&mut w);
		}
		u.script.call_method(cx.vm, "onSerialize", (cx.stream.clone(),))?;
	}
	Ok(())
}

/// Write the campaign's body (after the world state's `onBeforeSave` filled the
/// metadata) with the world state's `onSave` callback.
pub(crate) fn save(world: &dyn OriginalWorld, cx: &SaveCx, after: Option<SaveCallback>) -> Result<(), String> {
	let vm = cx.vm;
	let scene = world.scene(vm)?;
	{
		let mut w = cx.writer.borrow_mut();
		w.f64(world.virtual_time());
		scene.write(&mut w);
		world.camera().write(&mut w);
		world.clock().write(&mut w);
	}
	drop(scene);
	let rosters = world.rosters(vm)?;
	write_units(cx, &rosters.player)?;
	write_units(cx, &rosters.guests)?;
	cx.writer.borrow_mut().u16(rosters.created.len() as u16);
	for (id, units) in &rosters.created {
		cx.writer.borrow_mut().u32(*id);
		write_units(cx, units)?;
	}
	for (id, script) in world.entity_scripts() {
		cx.writer.borrow_mut().u32(id);
		script.call_method(vm, "onSerialize", (cx.stream.clone(),))?;
	}
	if let Some(after) = after {
		after.run(vm, &cx.stream);
	}
	fog::write(&mut cx.writer.borrow_mut(), &world.fog());
	Ok(())
}
