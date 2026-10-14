//! World entities' share of a save.
//!
//! Each live entity, by id, is written as its script path, id and native state (tile,
//! position, flags, sprites, labels), followed by whatever its script `onSerialize`
//! writes. Loading replaces every entity: each is re-created from its script with its
//! saved id (`create`), runs `onInit` (which re-registers it with the managers and
//! re-adds its sprites and thinker, as in the original), gets its saved native state
//! back, then runs `onDeserialize`.

use std::cell::RefCell;
use std::rc::Rc;

use squirrel::{SQVM, SqStrongRef};

use super::{EntityMachine, LabelData, SpriteData, create_entity, think};
use crate::native::save::stream::{SaveReader, SaveWriter};
use crate::native::save::{LoadCx, SaveCx, SavePart};

pub(crate) struct EntitiesPart(pub Rc<RefCell<EntityMachine>>);

/// The native flags of an entity, as saved.
struct Flags {
	pos: (f64, f64),
	faction: i64,
	rendered_top: bool,
	visibility: f32,
	visible: bool,
	discovered: bool,
	visible_in_fog_of_war: bool,
	ignore_collision: bool,
	important: bool,
	z_offset: i64,
}

impl Flags {
	fn write(&self, w: &mut SaveWriter) {
		w.f64(self.pos.0);
		w.f64(self.pos.1);
		w.i64(self.faction);
		w.f32(self.visibility);
		w.i64(self.z_offset);
		for b in [
			self.rendered_top,
			self.visible,
			self.discovered,
			self.visible_in_fog_of_war,
			self.ignore_collision,
			self.important,
		] {
			w.bool(b);
		}
	}

	fn read(r: &mut SaveReader) -> Result<Self, String> {
		Ok(Flags {
			pos: (r.f64()?, r.f64()?),
			faction: r.i64()?,
			visibility: r.f32()?,
			z_offset: r.i64()?,
			rendered_top: r.bool()?,
			visible: r.bool()?,
			discovered: r.bool()?,
			visible_in_fog_of_war: r.bool()?,
			ignore_collision: r.bool()?,
			important: r.bool()?,
		})
	}
}

fn write_sprite(w: &mut SaveWriter, s: &SpriteData) {
	w.str(&s.brush);
	// The colour without any fade in progress, which a load drops.
	w.u32(s.color.color);
	w.f64(s.scale);
	for b in [s.visible, s.ignore_ambient] {
		w.bool(b);
	}
	w.f64(s.offset.0);
	w.f64(s.offset.1);
	for b in [s.flipped, s.scaling, s.colorization, s.in_portrait] {
		w.bool(b);
	}
	w.f32(s.rotation);
	w.f32(s.spin.torque);
	w.u32(s.brush_bits);
}

fn read_sprite(r: &mut SaveReader) -> Result<SpriteData, String> {
	Ok(SpriteData {
		brush: r.str()?,
		color: crate::native::sprite_color::SpriteColor::new(r.u32()?),
		scale: r.f64()?,
		visible: r.bool()?,
		ignore_ambient: r.bool()?,
		offset: (r.f64()?, r.f64()?),
		flipped: r.bool()?,
		scaling: r.bool()?,
		colorization: r.bool()?,
		in_portrait: r.bool()?,
		rotation: r.f32()?,
		spin: crate::native::spin::Spin::new(r.f32()?),
		anim: Default::default(),
		brush_bits: r.u32()?,
	})
}

/// Sorted `(name, handle)` pairs, for a stable layout.
fn sorted<'a>(map: impl IntoIterator<Item = (&'a String, &'a SqStrongRef)>) -> Vec<(String, SqStrongRef)> {
	let mut v: Vec<(String, SqStrongRef)> = map.into_iter().map(|(k, h)| (k.clone(), h.clone())).collect();
	v.sort_by(|a, b| a.0.cmp(&b.0));
	v
}

impl SavePart for EntitiesPart {
	fn save(&self, cx: &SaveCx) -> Result<(), String> {
		let vm = cx.vm;
		let (order, next_id) = {
			let m = self.0.borrow();
			// An entity fading out to die is as good as gone.
			let mut order: Vec<usize> = (0..m.entities.len())
				.filter(|&i| m.entities[i].alive && m.entities[i].dies_at.is_none())
				.collect();
			order.sort_by_key(|&i| m.entities[i].id);
			(order, m.ids.peek())
		};
		cx.writer.borrow_mut().u32(order.len() as u32);
		for idx in order {
			let entity = {
				let m = self.0.borrow();
				let rec = &m.entities[idx];
				let sprite_class = m.sprite_class.clone().ok_or("save: sprite class missing")?;
				let label_class = m.label_class.clone().ok_or("save: label class missing")?;
				let mut w = cx.writer.borrow_mut();
				w.str(&rec.path);
				w.i64(rec.id);
				w.i64(rec.q);
				w.i64(rec.r);
				Flags {
					pos: rec.pos,
					faction: rec.faction,
					rendered_top: rec.rendered_top,
					visibility: rec.visibility,
					visible: rec.visible,
					discovered: rec.discovered,
					visible_in_fog_of_war: rec.visible_in_fog_of_war,
					ignore_collision: rec.ignore_collision,
					important: rec.important,
					z_offset: rec.z_offset,
				}
				.write(&mut w);
				// In layer order, so sprites a load adds keep their place.
				let sprites: Vec<(String, SqStrongRef)> =
					rec.sprites.iter().map(|(n, h)| (n.clone(), h.clone())).collect();
				w.u32(sprites.len() as u32);
				for (name, handle) in sprites {
					w.str(&name);
					sprite_class
						.with_payload(vm, &handle, |s| write_sprite(&mut w, s))
						.ok_or("save: not a sprite")?;
				}
				let labels = sorted(&rec.labels);
				w.u32(labels.len() as u32);
				for (name, handle) in labels {
					w.str(&name);
					label_class
						.with_payload(vm, &handle, |l| {
							w.bytes(&l.text);
							w.bool(l.visible);
							w.u32(l.color);
							w.f64(l.offset.0);
							w.f64(l.offset.1);
						})
						.ok_or("save: not a label")?;
				}
				rec.entity.clone()
			};
			entity.call_method(vm, "onSerialize", (cx.stream.clone(),))?;
		}
		cx.writer.borrow_mut().i64(next_id);
		Ok(())
	}

	fn load(&self, cx: &LoadCx) -> Result<(), String> {
		let vm = cx.vm;
		{
			let mut m = self.0.borrow_mut();
			m.entities.clear();
			m.by_identity.clear();
			m.by_id.clear();
			m.thinkers = think::Thinkers::default();
		}
		let count = cx.reader.borrow_mut().u32()?;
		for _ in 0..count {
			let (path, id, q, r, flags) = {
				let mut rd = cx.reader.borrow_mut();
				(rd.str()?, rd.i64()?, rd.i64()?, rd.i64()?, Flags::read(&mut rd)?)
			};
			let (entity, idx) = create_entity(&self.0, vm, &path, id, q, r)?;
			apply(&self.0, idx, &flags);
			entity.call_method(vm, "onInit", ())?;
			// The saved state wins over what onInit set up.
			apply(&self.0, idx, &flags);
			restore_handles(&self.0, vm, idx, cx)?;
			entity.call_method(vm, "onDeserialize", (cx.stream.clone(),))?;
		}
		let next_id = cx.reader.borrow_mut().i64()?;
		self.0.borrow().ids.restore(next_id);
		Ok(())
	}
}

/// Put an entity's saved native flags back.
fn apply(m: &Rc<RefCell<EntityMachine>>, idx: usize, f: &Flags) {
	let mut m = m.borrow_mut();
	let rec = &mut m.entities[idx];
	rec.pos = f.pos;
	rec.faction = f.faction;
	rec.rendered_top = f.rendered_top;
	rec.visibility = f.visibility;
	rec.visible = f.visible;
	rec.discovered = f.discovered;
	rec.visible_in_fog_of_war = f.visible_in_fog_of_war;
	rec.ignore_collision = f.ignore_collision;
	rec.important = f.important;
	rec.z_offset = f.z_offset;
}

/// Read an entity's saved sprites and labels into its handles, creating any its
/// `onInit` didn't.
fn restore_handles(m: &Rc<RefCell<EntityMachine>>, vm: &SQVM, idx: usize, cx: &LoadCx) -> Result<(), String> {
	let (sprite_class, label_class) = {
		let m = m.borrow();
		(
			m.sprite_class.clone().ok_or("load: sprite class missing")?,
			m.label_class.clone().ok_or("load: label class missing")?,
		)
	};
	let sprite_count = cx.reader.borrow_mut().u32()?;
	for _ in 0..sprite_count {
		let (name, data) = {
			let mut rd = cx.reader.borrow_mut();
			(rd.str()?, read_sprite(&mut rd)?)
		};
		let existing = m.borrow().entities[idx].sprites.get(&name).cloned();
		match existing {
			Some(handle) => {
				sprite_class.with_payload(vm, &handle, |s| *s = data);
			}
			None => {
				let handle = sprite_class.instantiate(vm, data);
				m.borrow_mut().entities[idx].sprites.insert(name, handle);
			}
		}
	}
	let label_count = cx.reader.borrow_mut().u32()?;
	for _ in 0..label_count {
		let data;
		let name = {
			let mut rd = cx.reader.borrow_mut();
			let name = rd.str()?;
			data = LabelData {
				text: rd.bytes()?,
				visible: rd.bool()?,
				color: rd.u32()?,
				offset: (rd.f64()?, rd.f64()?),
			};
			name
		};
		let existing = m.borrow().entities[idx].labels.get(&name).cloned();
		match existing {
			Some(handle) => {
				label_class.with_payload(vm, &handle, |l| *l = data);
			}
			None => {
				let handle = label_class.instantiate(vm, data);
				m.borrow_mut().entities[idx].labels.insert(name, handle);
			}
		}
	}
	Ok(())
}
