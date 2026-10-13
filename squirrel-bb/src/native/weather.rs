//! `World.getWeather()` / `Tactical.getWeather()`: clouds, birds, rain and lighting.
//!
//! All of it is visual, and building it draws nothing from the script RNG (probed for
//! the world 2026-10-13), so headless the builders are no-ops. The settings objects only
//! receive field writes from scripts, so each is a table pre-filled with every field
//! vanilla writes (an absent key can't be assigned); rain settings also carry their
//! drop-brush methods. `IsRaining` stays false: it only gates particle effects.

use squirrel::{Retained, SQVM, SqStrongRef, SqTable, Variadic};

const CLOUD: &[&str] = &[
	"Color", "Direction", "MaxAlpha", "MaxClouds", "MaxScale", "MaxVelocity", "MinAlpha", "MinClouds", "MinScale",
	"MinVelocity", "RandomizeDirection", "RandomizeRotation", "Sprite", "Type",
];
const BIRD: &[&str] = &["MaxBirds", "MaxVelocity", "MinBirds", "MinVelocity"];
const RAIN: &[&str] = &[
	"Direction", "DropLifetime", "MaxAlpha", "MaxDrops", "MaxScale", "MaxVelocity", "MinAlpha", "MinDrops", "MinScale",
	"MinVelocity", "NumSplats", "ScaleDropsWithTime", "SplatAlpha",
];
const RAIN_METHODS: &[&str] = &["clearDropBrushes", "addDropBrush"];
const BUILDERS: &[&str] = &[
	"buildCloudCover",
	"buildBirds",
	"buildRain",
	"setAmbientLightingPreset",
	"setAmbientLightingSaturation",
	"setAmbientLightingColor",
];

/// A fresh weather object.
pub(crate) fn weather(vm: &mut SQVM) -> SqStrongRef {
	let noop = vm.make_closure(|Variadic(_a): Variadic<Retained>| {});
	let creators: Vec<(&str, SqStrongRef)> = [
		("createCloudSettings", CLOUD, &[][..]),
		("createBirdSettings", BIRD, &[][..]),
		("createRainSettings", RAIN, RAIN_METHODS),
	]
	.into_iter()
	.map(|(name, fields, methods)| {
		let noop = noop.clone();
		let create = vm.make_closure(move |vm: &SQVM| {
			let mut t = SqTable::new(vm);
			for f in fields {
				t.set(f, 0.0);
			}
			for m in methods {
				t.set(m, noop.clone());
			}
			t.to_strong()
		});
		(name, create)
	})
	.collect();
	let mut w = SqTable::new(vm);
	w.set("IsRaining", false);
	for (name, create) in creators {
		w.set(name, create);
	}
	for name in BUILDERS {
		w.set(name, noop.clone());
	}
	w.to_strong()
}
