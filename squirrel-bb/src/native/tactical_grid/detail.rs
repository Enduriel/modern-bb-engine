//! The decal a battle tile's `spawnDetail` returns (blood, corpses, icons, terrain
//! dressing). Scripts set its look (`Scale`, `Alpha`, `Saturation`, `Color`,
//! `setBrightness`, `setBrush`); nothing reads it back for the game, and it has no
//! visual effect headless.

use squirrel::{ClassBuilder, SQVM, SqClass};

#[derive(Clone)]
pub(super) struct DetailData {
	brush: String,
	alpha: f64,
	scale: f64,
	saturation: f64,
	brightness: f64,
}

impl DetailData {
	pub fn fresh() -> Self {
		DetailData {
			brush: String::new(),
			alpha: 255.0,
			scale: 1.0,
			saturation: 1.0,
			brightness: 1.0,
		}
	}
}

pub(super) fn build_class(vm: &mut SQVM) -> SqClass<DetailData> {
	ClassBuilder::<DetailData>::new("TacticalDetail")
		.property("Alpha", |d: &DetailData| d.alpha, |d: &mut DetailData, v: f64| d.alpha = v)
		.property("Scale", |d: &DetailData| d.scale, |d: &mut DetailData, v: f64| d.scale = v)
		.property("Saturation", |d: &DetailData| d.saturation, |d: &mut DetailData, v: f64| d.saturation = v)
		.property_noop("Color")
		.method("setBrightness", |d: &mut DetailData, v: f64| d.brightness = v)
		.method("setBrush", |d: &mut DetailData, brush: String| d.brush = brush)
		.build(vm)
}
