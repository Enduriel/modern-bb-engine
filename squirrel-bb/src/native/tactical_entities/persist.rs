//! The rosters' share of a save: every roster with its units, in roster order.
//!
//! A unit is written as its script path, id and native state (faction, flags, position,
//! sprites), followed by whatever its script `onSerialize` writes. Loading replaces all
//! rosters: each unit is re-created from its script with its saved id (`create`), runs
//! `onInit` (setting up its properties and default skills, which `onDeserialize`'s
//! non-stacking adds then refresh), gets its native state back, then runs
//! `onDeserialize`, like the world entities.

use std::cell::RefCell;
use std::rc::Rc;

use squirrel::{SQVM, SqStrongRef};

use super::{TacticalMachine, UnitSpriteData, create_unit};
use crate::native::save::stream::{SaveReader, SaveWriter};
use crate::native::save::{LoadCx, SaveCx, SavePart};
use crate::native::sprite_color::SpriteColor;

pub(crate) struct RostersPart(pub Rc<RefCell<TacticalMachine>>);

fn write_sprite(w: &mut SaveWriter, s: &UnitSpriteData) {
	w.str(&s.brush);
	w.u32(s.color.color);
	w.f64(s.offset.0);
	w.f64(s.offset.1);
	for v in [s.color.alpha() as f64, s.scale, s.rotation, s.saturation, s.render_anim_start] {
		w.f64(v);
	}
	w.f64(s.highlight_factor);
	w.u32(s.highlight);
	w.f64(f64::from(s.spin.torque));
	for b in [
		s.visible,
		s.raising_weapon,
		s.lowering_weapon,
		s.raising_shield,
		s.lowering_shield,
		s.flipped,
		s.finished,
		s.fading,
		s.color.is_fading_done(),
		s.ignore_ambient,
		s.ignore_camera_flip,
	] {
		w.bool(b);
	}
	w.bool(s.in_portrait);
	w.bool(s.occlusion.is_some());
	if let Some((a, b, c)) = s.occlusion {
		w.i64(a);
		w.i64(b);
		w.i64(c);
	}
	w.u32(s.brush_bits);
}

/// A sprite's fade isn't kept: it loads with the colour it had.
fn read_sprite(r: &mut SaveReader) -> Result<UnitSpriteData, String> {
	let brush = r.str()?;
	let mut color = SpriteColor::new(r.u32()?);
	let offset = (r.f64()?, r.f64()?);
	color.set_alpha(r.f64()? as i64);
	Ok(UnitSpriteData {
		brush,
		color,
		offset,
		scale: r.f64()?,
		rotation: r.f64()?,
		saturation: r.f64()?,
		render_anim_start: r.f64()?,
		highlight_factor: r.f64()?,
		highlight: r.u32()?,
		stored: super::shaker::Stored::default(),
		is_stored: false,
		spin: crate::native::spin::Spin::new(r.f64()? as f32),
		visible: r.bool()?,
		raising_weapon: r.bool()?,
		lowering_weapon: r.bool()?,
		raising_shield: r.bool()?,
		lowering_shield: r.bool()?,
		flipped: r.bool()?,
		finished: r.bool()?,
		fading: r.bool()?,
		ignore_ambient: {
			r.bool()?; // IsFadingDone, which follows the colour
			r.bool()?
		},
		ignore_camera_flip: r.bool()?,
		in_portrait: r.bool()?,
		occlusion: if r.bool()? { Some((r.i64()?, r.i64()?, r.i64()?)) } else { None },
		brush_bits: r.u32()?,
	})
}

impl SavePart for RostersPart {
	fn save(&self, cx: &SaveCx) -> Result<(), String> {
		let vm = cx.vm;
		let (rosters, next_id) = {
			let m = self.0.borrow();
			let mut rosters: Vec<(i64, Vec<SqStrongRef>)> =
				m.rosters.iter().map(|(id, units)| (*id, units.clone())).collect();
			rosters.sort_by_key(|(id, _)| *id);
			(rosters, m.ids.peek())
		};
		cx.writer.borrow_mut().u32(rosters.len() as u32);
		for (roster_id, units) in rosters {
			{
				let mut w = cx.writer.borrow_mut();
				w.i64(roster_id);
				w.u32(units.len() as u32);
			}
			for unit in units {
				{
					let m = self.0.borrow();
					let rec = m.record(unit.identity()).ok_or("save: unit not registered")?;
					let sprite_class = m.sprite_class.clone().ok_or("save: sprite class missing")?;
					let mut w = cx.writer.borrow_mut();
					w.str(&rec.path);
					w.i64(rec.id);
					w.i64(rec.faction);
					w.bool(rec.discovered);
					w.bool(rec.blocks_sight);
					w.f64(rec.pos.0);
					w.f64(rec.pos.1);
					// In layering order.
					let sprites: Vec<(&String, &SqStrongRef)> = rec.sprites.iter().collect();
					w.u32(sprites.len() as u32);
					for (name, handle) in sprites {
						w.str(name);
						sprite_class
							.with_payload(vm, handle, |s| write_sprite(&mut w, s))
							.ok_or("save: not a unit sprite")?;
					}
				}
				unit.call_method(vm, "onSerialize", (cx.stream.clone(),))?;
			}
		}
		cx.writer.borrow_mut().i64(next_id);
		Ok(())
	}

	fn load(&self, cx: &LoadCx) -> Result<(), String> {
		let vm = cx.vm;
		{
			let mut m = self.0.borrow_mut();
			m.rosters.clear();
			m.units.clear();
			m.by_identity.clear();
		}
		let roster_count = cx.reader.borrow_mut().u32()?;
		for _ in 0..roster_count {
			let (roster_id, unit_count) = {
				let mut r = cx.reader.borrow_mut();
				(r.i64()?, r.u32()?)
			};
			self.0.borrow_mut().rosters.entry(roster_id).or_default();
			for _ in 0..unit_count {
				load_unit(&self.0, vm, roster_id, cx)?;
			}
		}
		let next_id = cx.reader.borrow_mut().i64()?;
		self.0.borrow().ids.restore(next_id);
		Ok(())
	}
}

fn load_unit(m: &Rc<RefCell<TacticalMachine>>, vm: &SQVM, roster_id: i64, cx: &LoadCx) -> Result<(), String> {
	let (path, id, faction, discovered, blocks_sight, pos) = {
		let mut r = cx.reader.borrow_mut();
		(r.str()?, r.i64()?, r.i64()?, r.bool()?, r.bool()?, (r.f64()?, r.f64()?))
	};
	let (unit, idx) = create_unit(m, vm, &path, id)?;
	m.borrow_mut().units[idx].faction = faction;
	unit.call_method(vm, "onInit", ())?;
	{
		let mut mm = m.borrow_mut();
		let rec = &mut mm.units[idx];
		rec.initialized = true;
		rec.faction = faction;
		rec.discovered = discovered;
		rec.blocks_sight = blocks_sight;
		rec.pos = pos;
	}
	let sprite_class = m.borrow().sprite_class.clone().ok_or("load: sprite class missing")?;
	let sprite_count = cx.reader.borrow_mut().u32()?;
	for _ in 0..sprite_count {
		let (name, data) = {
			let mut r = cx.reader.borrow_mut();
			(r.str()?, read_sprite(&mut r)?)
		};
		let existing = m.borrow().units[idx].sprites.get(&name).cloned();
		match existing {
			Some(handle) => {
				sprite_class.with_payload(vm, &handle, |s| *s = data);
			}
			None => {
				let handle = sprite_class.instantiate(vm, data);
				m.borrow_mut().units[idx].sprites.insert(name, handle);
			}
		}
	}
	unit.call_method(vm, "onDeserialize", (cx.stream.clone(),))?;
	m.borrow_mut().rosters.entry(roster_id).or_default().push(unit);
	Ok(())
}
