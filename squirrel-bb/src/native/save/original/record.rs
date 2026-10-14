//! A scene entity as the original saves it: the record every entity starts with
//! (`0x542500` writes, `0x542350` reads), then its class's part.
//!
//! A class's type id is `bb_assets::name_hash` of its type name ([`type_id`]). Classes
//! that occur in a campaign save: `WorldEntity` (world parties, locations,
//! settlements), `WorldDecal` (the map's decorations), `WorldFootprint`, `TextEntity`
//! (the region names) in the world scene, and `TacticalEntity` (roster units, whose
//! script data follows their native part inline; see [`TacticalNative`]).

use super::super::stream::{SaveReader, SaveWriter};
use super::game_entity::GameEntity;
use super::sprite::Sprite;
use super::wire;

/// A class's type id: the hash of its type name.
pub(crate) fn type_id(name: &str) -> u64 {
	bb_assets::name_hash(name.as_bytes())
}

/// The record's flags: the entity is in a scene, and is saved. Only entities with both
/// are written.
pub(crate) const SAVED: u32 = 0x401;
/// The entity's children are saved.
const SAVES_CHILDREN: u32 = 0x800;

/// What every entity's record starts with.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Head {
	pub id: u32,
	pub pos: (f32, f32),
	/// Z (`setZ`).
	pub z: i32,
	/// Unnamed (`+0x14`).
	pub i14: i32,
	/// Degrees.
	pub rotation: f32,
	/// Scene flags (`+0x30`).
	pub flags: u32,
	/// Unnamed (`+0x34`).
	pub u34: u32,
	pub children: Vec<SceneEntity>,
}

impl Head {
	pub fn read(r: &mut SaveReader) -> Result<Self, String> {
		let mut head = Head {
			id: r.u32()?,
			pos: wire::vec2(r)?,
			z: wire::i32(r)?,
			i14: wire::i32(r)?,
			rotation: r.f32()?,
			flags: r.u32()?,
			u34: r.u32()?,
			children: Vec::new(),
		};
		for _ in 0..r.u8()? {
			head.children.push(SceneEntity::read(r)?);
		}
		Ok(head)
	}

	/// As the original writes it: the child count is of the children flagged 0x400,
	/// while every child gets its type id (and the saved ones their record).
	pub fn write(&self, w: &mut SaveWriter) {
		w.u32(self.id);
		wire::write_vec2(w, self.pos);
		w.u32(self.z as u32);
		w.u32(self.i14 as u32);
		w.f32(self.rotation);
		w.u32(self.flags);
		w.u32(self.u34);
		if self.flags & SAVES_CHILDREN == 0 || self.children.is_empty() {
			w.u8(0);
			return;
		}
		w.u8(self.children.iter().filter(|c| c.head.flags & 0x400 != 0).count() as u8);
		for c in &self.children {
			c.write(w);
		}
	}
}

/// A world entity's class part (`0x4e7f10` writes, `0x4e7dc0` reads). Its script data
/// is saved later, by id (see the campaign layout in [`super`]).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct WorldEntityPart {
	pub discovered: bool,
	pub ignore_collision: bool,
	pub important: bool,
	/// The script's `ClassNameHash`, which the load creates the script from.
	pub script_hash: i32,
	pub game: GameEntity,
}

impl WorldEntityPart {
	fn read(r: &mut SaveReader) -> Result<Self, String> {
		Ok(WorldEntityPart {
			discovered: r.bool()?,
			ignore_collision: r.bool()?,
			important: r.bool()?,
			script_hash: wire::i32(r)?,
			game: GameEntity::read(r)?,
		})
	}

	fn write(&self, w: &mut SaveWriter) {
		w.bool(self.discovered);
		w.bool(self.ignore_collision);
		w.bool(self.important);
		w.u32(self.script_hash as u32);
		self.game.write(w);
	}
}

/// A unit's native class part (`0x4c37f0`), which its script's `onSerialize` follows.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TacticalNative {
	pub script_hash: i32,
	pub game: GameEntity,
}

impl TacticalNative {
	pub fn read(r: &mut SaveReader) -> Result<Self, String> {
		Ok(TacticalNative {
			script_hash: wire::i32(r)?,
			game: GameEntity::read(r)?,
		})
	}

	pub fn write(&self, w: &mut SaveWriter) {
		w.u32(self.script_hash as u32);
		self.game.write(w);
	}
}

/// A label the original draws as a scene entity: the world's region names (`0x561fe0`).
/// Only the font and the text are traced; the rest is named from the label properties
/// the original registers and from the values seen.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Text {
	pub font: Vec<u8>,
	pub offset: (f32, f32),
	/// 0.2, 0.3 or 0.4 for region names.
	pub size: f32,
	pub alpha: f32,
	pub valign: u8,
	pub halign: u8,
	pub color: u32,
	pub color2: u32,
	pub b9c: bool,
	pub text: Vec<u8>,
}

impl Text {
	fn read(r: &mut SaveReader) -> Result<Self, String> {
		Ok(Text {
			font: r.bytes()?,
			offset: wire::vec2(r)?,
			size: r.f32()?,
			alpha: r.f32()?,
			valign: r.u8()?,
			halign: r.u8()?,
			color: r.u32()?,
			color2: r.u32()?,
			b9c: r.bool()?,
			text: r.bytes()?,
		})
	}

	fn write(&self, w: &mut SaveWriter) {
		w.bytes(&self.font);
		wire::write_vec2(w, self.offset);
		w.f32(self.size);
		w.f32(self.alpha);
		w.u8(self.valign);
		w.u8(self.halign);
		w.u32(self.color);
		w.u32(self.color2);
		w.bool(self.b9c);
		w.bytes(&self.text);
	}
}

/// A class's part of a scene entity's record.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Body {
	World(WorldEntityPart),
	/// A decal, a wave, a plane or a battle's sprite effect: its sprite.
	Sprite(Sprite),
	/// A party's footprint: its sprite, when it was left, its kind.
	Footprint { sprite: Sprite, time: f32, kind: u16 },
	Text(Text),
	/// Classes that save nothing of their own (birds, clouds, particles, lines).
	Nothing,
}

/// The classes whose part is a sprite.
const SPRITE_CLASSES: [&str; 14] = [
	"WorldDecal",
	"WorldWaveSprite",
	"Plane",
	"TacticalAttackSprite",
	"CloudSprite",
	"Decal",
	"FadeAndDieSprite",
	"FireAndForgetSprite",
	"TacticalFootprint",
	"TacticalIconSprite",
	"MirrorPlane",
	"PoolSprite",
	"ProjectileSprite",
	"RainSprite",
];
const EMPTY_CLASSES: [&str; 5] = ["WorldBirdSprite", "WorldCloudSprite", "ParticleEmitter", "Lines", "TacticalHeadSprite"];

impl Body {
	fn read(type_: u64, r: &mut SaveReader) -> Result<Self, String> {
		let is = |names: &[&str]| names.iter().any(|n| type_id(n) == type_);
		if type_ == type_id("WorldEntity") {
			return Ok(Body::World(WorldEntityPart::read(r)?));
		}
		if type_ == type_id("WorldFootprint") {
			return Ok(Body::Footprint {
				sprite: Sprite::read(r)?,
				time: r.f32()?,
				kind: r.u16()?,
			});
		}
		if type_ == type_id("TextEntity") {
			return Ok(Body::Text(Text::read(r)?));
		}
		if is(&SPRITE_CLASSES) {
			return Ok(Body::Sprite(Sprite::read(r)?));
		}
		if is(&EMPTY_CLASSES) {
			return Ok(Body::Nothing);
		}
		Err(format!("a scene entity of an unknown class ({type_:#018x})"))
	}

	fn write(&self, w: &mut SaveWriter) {
		match self {
			Body::World(e) => e.write(w),
			Body::Sprite(s) => s.write(w),
			Body::Footprint { sprite, time, kind } => {
				sprite.write(w);
				w.f32(*time);
				w.u16(*kind);
			}
			Body::Text(t) => t.write(w),
			Body::Nothing => {}
		}
	}
}

/// A scene entity, with its type id, as a layer (or a parent) saves it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SceneEntity {
	pub type_: u64,
	pub head: Head,
	pub body: Body,
}

impl SceneEntity {
	pub fn read(r: &mut SaveReader) -> Result<Self, String> {
		let type_ = r.i64()? as u64;
		let head = Head::read(r)?;
		let body = Body::read(type_, r)?;
		Ok(SceneEntity { type_, head, body })
	}

	pub fn write(&self, w: &mut SaveWriter) {
		w.i64(self.type_ as i64);
		if self.head.flags & SAVED != SAVED {
			return;
		}
		self.head.write(w);
		self.body.write(w);
	}
}
