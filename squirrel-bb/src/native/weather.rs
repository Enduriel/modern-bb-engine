//! `World.getWeather()` / `Tactical.getWeather()`: clouds, birds, rain and lighting.
//!
//! All of it is visual, and building it draws nothing from the script RNG (probed for
//! the world), so headless the builders are no-ops, except the world's
//! `buildCloudCover`, whose clouds are kept for the renderer (`world_clouds`). The settings
//! objects only receive field writes from scripts, so each is a table pre-filled with
//! every field the original's has, at the value a fresh one reads there (probed;
//! an absent key can't be assigned); rain settings also carry their
//! drop-brush methods. The original's are class instances. `IsRaining` stays false: it
//! only gates particle effects.

use squirrel::{SQVM, SqStrongRef, SqTable};

/// A settings field's starting value.
#[derive(Clone, Copy)]
enum Field {
	Float(f64),
	Int(i64),
	Bool(bool),
	Text(&'static str),
	/// ARGB.
	Color(u32),
	Vector(f64, f64),
}

use Field::*;

const CLOUD: &[(&str, Field)] = &[
	("Color", Color(0xffff_ffff)),
	("Direction", Vector(0.430968, 0.902367)),
	("MaxAlpha", Float(1.0)),
	("MaxClouds", Int(1)),
	("MaxScale", Float(1.0)),
	("MaxVelocity", Float(0.0)),
	("MinAlpha", Float(1.0)),
	("MinClouds", Int(1)),
	("MinScale", Float(1.0)),
	("MinVelocity", Float(0.0)),
	("RandomizeDirection", Bool(true)),
	("RandomizeRotation", Bool(true)),
	("Sprite", Text("")),
	("Type", Int(0)),
];
const BIRD: &[(&str, Field)] =
	&[("MaxBirds", Int(1)), ("MaxVelocity", Float(0.0)), ("MinBirds", Int(1)), ("MinVelocity", Float(0.0))];
const RAIN: &[(&str, Field)] = &[
	("Direction", Vector(-0.3, -0.7)),
	("DropLifetime", Int(600)),
	("MaxAlpha", Float(1.0)),
	("MaxDrops", Int(1)),
	("MaxScale", Float(1.0)),
	("MaxVelocity", Float(1.0)),
	("MinAlpha", Float(1.0)),
	("MinDrops", Int(1)),
	("MinScale", Float(1.0)),
	("MinVelocity", Float(1.0)),
	("NumSplats", Int(0)),
	("ScaleDropsWithTime", Bool(false)),
	("SplatAlpha", Float(1.0)),
	("SplatBrush", Text("rainsplat_01")),
];

/// Set field `name` of `t` to its starting value.
fn set_field(vm: &SQVM, t: &mut SqTable, name: &str, field: Field) {
	match field {
		Float(v) => t.set(name, v),
		Int(v) => t.set(name, v),
		Bool(v) => t.set(name, v),
		Text(v) => t.set(name, v.to_string()),
		Color(argb) => {
			if let Some(c) = super::color::color_object(vm, argb) {
				t.set(name, c);
			}
		}
		Vector(x, y) => {
			let v = SqTable::root(vm).get::<SqStrongRef>("createVec").and_then(|f| f.call_ret::<SqStrongRef, _>(vm, (x, y)).ok().flatten());
			if let Some(v) = v {
				t.set(name, v);
			}
		}
	}
}
const RAIN_METHODS: &[&str] = &["clearDropBrushes", "addDropBrush"];
const BUILDERS: &[&str] = &[
	"buildCloudCover",
	"buildBirds",
	"buildRain",
	"setAmbientLightingPreset",
	"setAmbientLightingSaturation",
	"setAmbientLightingColor",
	"clear",
];

/// The enums the original registers in the const table with its weather class.
const AMBIENT_LIGHTING_PRESET: &[(&str, i64)] = &[
	("Default", 0),
	("Dawn", 1),
	("Dusk", 2),
	("Night", 3),
	("Midday", 4),
	("Rain", 5),
	("Storm", 6),
	("Desert", 7),
];
const CLOUD_TYPE: &[(&str, i64)] = &[("Default", 0), ("Fog", 1), ("StaticFog", 2), ("SunHighlights", 3), ("Custom", 4)];

/// Register `AmbientLightingPreset` and `CloudType` in the const table (values from the
/// original binary's weather binding).
pub(crate) fn install_consts(vm: &SQVM) {
	for (name, members) in [("AmbientLightingPreset", AMBIENT_LIGHTING_PRESET), ("CloudType", CLOUD_TYPE)] {
		let mut e = SqTable::new(vm);
		for (k, v) in members {
			e.set(k, *v);
		}
		let e = e.to_strong();
		SqTable::const_table(vm).set(name, e);
	}
}

/// A fresh weather object for `scope` (`World` or `Tactical`; its stubs' names).
pub(crate) fn weather(vm: &mut SQVM, scope: &str) -> SqStrongRef {
	let creators: Vec<(&str, SqStrongRef)> = [
		("createCloudSettings", CLOUD, &[][..]),
		("createBirdSettings", BIRD, &[][..]),
		("createRainSettings", RAIN, RAIN_METHODS),
	]
	.into_iter()
	.map(|(name, fields, methods)| {
		let stubs: Vec<(&str, SqStrongRef)> = methods
			.iter()
			.map(|m| (*m, super::stubs::closure(vm, super::stubs::name(format!("{scope}.getWeather().{name}().{m}")))))
			.collect();
		let create = vm.make_closure(move |vm: &SQVM| {
			let mut t = SqTable::new(vm);
			for (name, field) in fields {
				set_field(vm, &mut t, name, *field);
			}
			for (m, stub) in &stubs {
				t.set(*m, stub.clone());
			}
			t.to_strong()
		});
		(name, create)
	})
	.collect();
	// Each builder is a stub until a real one replaces it (`world`, `tactical`).
	let builders: Vec<(&str, SqStrongRef)> =
		BUILDERS.iter().map(|name| (*name, super::stubs::closure(vm, super::stubs::name(format!("{scope}.getWeather().{name}"))))).collect();
	let mut w = SqTable::new(vm);
	w.set("IsRaining", false);
	for (name, create) in creators {
		w.set(name, create);
	}
	for (name, stub) in builders {
		w.set(name, stub);
	}
	w.to_strong()
}
