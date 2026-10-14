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

use squirrel::{ClassBuilder, Native, Retained, SQVM, SqTable, Variadic};

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

impl TileTransition {
	fn new() -> Self {
		TileTransition {
			socket: None,
			blend_into_sockets: true,
			brushes: Vec::new(),
		}
	}
}

/// Register the tile-transition class, the `createTileTransition` root global, and
/// the `Tactical` table; returns the transition registry.
pub(crate) fn install(vm: &mut SQVM) -> Transitions {
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

	// Tactical.getShaker() (hit shakes) and getHighlighter() (tile highlights, overlay
	// icons): visual only, and no script reads anything back from them.
	let noop = vm.make_closure(|Variadic(_a): Variadic<Retained>| {});
	let shaker = {
		let mut t = SqTable::new(vm);
		for name in ["shake", "cancel"] {
			t.set(name, noop.clone());
		}
		t.to_strong()
	};
	let get_shaker = vm.make_closure(move || shaker.clone());
	let highlighter = {
		let mut t = SqTable::new(vm);
		for name in ["addOverlayIcon", "clear", "clearOverlayIcons", "highlightRangeOfSkill", "highlightZoneOfControl"] {
			t.set(name, noop.clone());
		}
		t.to_strong()
	};
	let get_highlighter = vm.make_closure(move || highlighter.clone());
	// Tactical.getWeather(): visual only (see `weather`).
	super::weather::install_consts(vm);
	let weather = super::weather::weather(vm);
	let get_weather = vm.make_closure(move || weather.clone());
	let mut tactical = SqTable::new(vm);
	tactical.set("getWeather", get_weather);
	tactical.set("getShaker", get_shaker);
	tactical.set("getHighlighter", get_highlighter);
	tactical.set("setTransitions", set_transitions);
	tactical.set("setActive", set_active);
	tactical.set("isActive", is_active);
	tactical.set("setVisible", set_visible);
	tactical.set("isVisible", is_visible);
	SqTable::root(vm).set("Tactical", tactical);
	registry
}
