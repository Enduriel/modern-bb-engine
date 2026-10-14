//! Tactical-layer natives needed at construction time, plus the tile-transition
//! globals that sit beside them.
//!
//! `map_generator.create()` registers every map template and calls each one's
//! `init()`. The tactical tile templates (38 of them) build a tile-transition
//! descriptor there — `createTileTransition()` then `setBrush`/`setSocket`/
//! `setBlendIntoSockets` on it — and hand it to `Tactical.setTransitions`. None of
//! that can construct without these natives, so the object-graph load needs them.
//!
//! The [`TileTransition`] descriptor records its configuration, and
//! `Tactical.setTransitions` stores it in a registry keyed by tile-brush name, which
//! the tactical scene hands to the renderer.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use squirrel::{ClassBuilder, Native, Number, Retained, SQVM, SqStrongRef, SqTable, Variadic};

use super::world_clouds::{CloudCover, Clouds, MapSpan};

/// One tile type's transition configuration — the payload behind a
/// `createTileTransition()` instance, and what `Tactical.setTransitions` caches.
#[derive(Clone, Debug, PartialEq)]
pub struct TileTransition {
	/// Socket name (`setSocket`) — which transition socket this tile plugs into.
	pub socket: Option<String>,
	/// Whether the tile blends into neighbouring sockets (`setBlendIntoSockets`);
	/// defaults true (only a few tiles, e.g. roads, turn it off).
	pub blend_into_sockets: bool,
	/// Per-direction border brushes (`setBrush(direction, name)`), in call order.
	pub brushes: Vec<(i64, String)>,
}

/// The transition registry, by tile-brush name.
pub(crate) type Transitions = Rc<RefCell<HashMap<String, TileTransition>>>;

/// The battle's light: ambient colour (ARGB) and saturation, as the tactical weather's
/// `setAmbientLighting*` set them (`0x4e1280`, `0x4e12a0`, `0x4e12c0`: straight into
/// the battle scene's ambient).
pub(crate) type TacticalLight = Rc<std::cell::Cell<(u32, f32)>>;

/// `setAmbientLightingPreset(i)`'s colours, by `AmbientLightingPreset` (Default, Dawn,
/// Dusk, Night, Midday, Rain, Storm, Desert), from the table the original fills at
/// startup (`0x463b79`…).
const LIGHTING_PRESETS: [u32; 8] = [
	0xffff_ffff,
	0xffca_e7f8,
	0xfff8_c3be,
	0xff70_9de0,
	0xffff_ffff,
	0xffc3_cbd0,
	0xff9b_a5ad,
	0xfff5_f0e1,
];

impl TileTransition {
	fn new() -> Self {
		TileTransition {
			socket: None,
			blend_into_sockets: true,
			brushes: Vec::new(),
		}
	}
}

/// What the battle's natives keep for the engine.
pub(crate) struct TacticalParts {
	pub transitions: Transitions,
	pub light: TacticalLight,
	/// The battle's clouds (`Tactical.getWeather().buildCloudCover`), the world's kind
	/// over the battle map. `Tactical.update` moves them.
	pub clouds: Clouds,
	/// The battle's particle effects (`Tactical.spawnParticleEffect`).
	pub particles: super::particles::ParticlesHandle,
	/// The battle's other visual effects (`Tactical.spawn…Effect`, see
	/// `battle_effects`).
	pub effects: super::battle_effects::BattleEffectsHandle,
}

/// The first and last tile centres of the battle map (`Tactical.getMapSize()`), or
/// `None` without a map.
fn battle_span(vm: &SQVM) -> Result<Option<MapSpan>, String> {
	let tactical = SqTable::root(vm).get::<SqStrongRef>("Tactical").ok_or("Tactical missing")?;
	let size = tactical.call_method_ret::<SqStrongRef, _>(vm, "getMapSize", ())?;
	let dim = |axis: &str| size.as_ref().and_then(|s| s.get_member::<Number>(vm, axis)).map_or(0, |n| n.0 as i64);
	let (w, h) = (dim("X"), dim("Y"));
	if w <= 0 || h <= 0 {
		return Ok(None);
	}
	let (min, max) = (super::tactical_grid::tile_pos(0, 0), super::tactical_grid::tile_pos(w - 1, h - 1));
	Ok(Some(MapSpan {
		min: (min.0 as f32, min.1 as f32),
		max: (max.0 as f32, max.1 as f32),
	}))
}

/// The battle's effects over the rest of `Tactical`: `Tactical.update()` also moves the
/// clouds (by the frame's real time while the game runs, as the world's weather moves
/// with `World.update`) and the particle effects; `Tactical.clearScene()` also clears
/// them; `spawnParticleEffect` spawns into them. Call after whatever else the
/// `Tactical` table holds is installed.
pub(crate) fn install_effects(vm: &mut SQVM, parts: &TacticalParts) {
	let inner = SqTable::root(vm)
		.get::<SqTable>("Tactical")
		.and_then(|t| t.get::<SqStrongRef>("update"));
	let inner_clear = SqTable::root(vm)
		.get::<SqTable>("Tactical")
		.and_then(|t| t.get::<SqStrongRef>("clearScene"));
	let (clouds, particles) = (parts.clouds.clone(), parts.particles.clone());
	let clear = {
		let (clouds, particles, effects) = (clouds.clone(), particles.clone(), parts.effects.clone());
		vm.make_closure(move |vm: &SQVM| -> Result<(), String> {
			if let Some(inner) = &inner_clear {
				inner.call(vm, ())?;
			}
			clouds.borrow_mut().clear();
			particles.borrow_mut().clear();
			effects.borrow_mut().clear();
			Ok(())
		})
	};
	let update = vm.make_closure(move |vm: &SQVM| -> Result<(), String> {
		if let Some(inner) = &inner {
			inner.call(vm, ())?;
		}
		let time = SqTable::root(vm).get::<SqStrongRef>("Time").ok_or("Time missing")?;
		let delta = time.call_method_ret::<Number, _>(vm, "getDelta", ())?.map_or(0.0, |n| n.0);
		let speed = time.call_method_ret::<Number, _>(vm, "getVirtualSpeed", ())?.map_or(1.0, |n| n.0);
		clouds.borrow_mut().advance(delta as f32, speed as f32);
		super::particles::advance_now(vm, &particles, delta as f32);
		Ok(())
	});
	if let Some(mut tactical) = SqTable::root(vm).get::<SqTable>("Tactical") {
		tactical.set("update", update);
		tactical.set("clearScene", clear);
	}
	super::particles::install_on(vm, "Tactical", &parts.particles);
}

/// Register the tile-transition class, the `createTileTransition` root global, and
/// the `Tactical` table.
pub(crate) fn install(vm: &mut SQVM, engine_rng: &super::EngineRng, crt: &super::prng::Prng) -> TacticalParts {
	// The descriptor class: three mutating setters over the payload.
	let transition = ClassBuilder::<TileTransition>::new("TileTransition")
		.method("setSocket", |t: &mut TileTransition, name: String| {
			t.socket = Some(name);
		})
		.method(
			"setBlendIntoSockets",
			|t: &mut TileTransition, blend: bool| {
				t.blend_into_sockets = blend;
			},
		)
		.method(
			"setBrush",
			|t: &mut TileTransition, direction: i64, name: String| {
				t.brushes.push((direction, name));
			},
		)
		.build(vm);

	// createTileTransition() -> a fresh descriptor instance.
	vm.register("createTileTransition", move |vm: &SQVM| {
		transition.instantiate(vm, TileTransition::new())
	});

	// Tactical.setTransitions(tileBrushName, transition): keep the descriptor for the
	// renderer. The registry holds plain Rust data (no VM handles), so it has no
	// teardown ordering concerns.
	let registry: Transitions = Rc::new(RefCell::new(HashMap::new()));
	let set_transitions = {
		let registry = registry.clone();
		vm.make_closure(move |name: String, transition: Native<TileTransition>| {
			registry.borrow_mut().insert(name, transition.0);
		})
	};

	// Whether tactical mode is on (setActive) and its scene is showing (setVisible), as
	// the tactical/world states set them. Both are off on the world map.
	let active = Rc::new(std::cell::Cell::new(false));
	let visible = Rc::new(std::cell::Cell::new(false));
	let set_active = {
		let a = active.clone();
		vm.make_closure(move |on: bool| a.set(on))
	};
	let is_active = vm.make_closure(move || active.get());
	let set_visible = {
		let v = visible.clone();
		vm.make_closure(move |on: bool| v.set(on))
	};
	let is_visible = vm.make_closure(move || visible.get());

	// Tactical.getCamera() comes with the battle map (see `tactical_grid::camera`).

	// Tactical.getShaker() comes with the units (see `tactical_entities::shaker`), and
	// Tactical.getHighlighter() with the map (see `tactical_highlighter`).
	// Tactical.getWeather(): visual only (see `weather`).
	super::weather::install_consts(vm);
	let weather = super::weather::weather(vm, "Tactical");
	let light: TacticalLight = Rc::new(std::cell::Cell::new((0xffff_ffff, 1.0)));
	let set_color = {
		let light = light.clone();
		vm.make_closure(move |vm: &SQVM, color: Retained| -> Result<(), String> {
			let argb = super::color::color_argb(vm, &color.0).ok_or("setAmbientLightingColor: not a color")?;
			light.set((argb, light.get().1));
			Ok(())
		})
	};
	let set_saturation = {
		let light = light.clone();
		vm.make_closure(move |s: squirrel::Number| light.set((light.get().0, s.0 as f32)))
	};
	let set_preset = {
		let light = light.clone();
		vm.make_closure(move |preset: i64| {
			if let Some(&argb) = usize::try_from(preset).ok().and_then(|i| LIGHTING_PRESETS.get(i)) {
				light.set((argb, light.get().1));
			}
		})
	};
	let clouds: Clouds = Rc::new(RefCell::new(CloudCover::new(engine_rng.clone())));
	let build_cloud_cover = {
		let clouds = clouds.clone();
		vm.make_closure(move |vm: &SQVM, settings: Retained| -> Result<(), String> {
			let settings = super::world::cloud_settings(vm, &settings.0);
			match battle_span(vm)? {
				Some(span) => clouds.borrow_mut().build(settings, span),
				None => clouds.borrow_mut().clear(),
			}
			Ok(())
		})
	};
	let clear = {
		let clouds = clouds.clone();
		vm.make_closure(move |Variadic(_a): Variadic<Retained>| clouds.borrow_mut().clear())
	};
	for (name, f) in [
		("setAmbientLightingColor", set_color),
		("setAmbientLightingSaturation", set_saturation),
		("setAmbientLightingPreset", set_preset),
		("buildCloudCover", build_cloud_cover),
		("clear", clear),
	] {
		weather.set_member(vm, name, f).expect("the weather table");
	}
	let get_weather = vm.make_closure(move || weather.clone());
	let mut tactical = SqTable::new(vm);
	tactical.set("getWeather", get_weather);
	tactical.set("setTransitions", set_transitions);
	tactical.set("setActive", set_active);
	tactical.set("isActive", is_active);
	tactical.set("setVisible", set_visible);
	tactical.set("isVisible", is_visible);
	SqTable::root(vm).set("Tactical", tactical);
	TacticalParts {
		transitions: registry,
		light,
		clouds,
		particles: Rc::new(RefCell::new(super::particles::Particles::new(engine_rng.clone()))),
		effects: super::battle_effects::BattleEffects::handle(engine_rng.clone(), crt.clone()),
	}
}
