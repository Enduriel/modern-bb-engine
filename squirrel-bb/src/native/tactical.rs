//! Tactical-layer natives needed at construction time, plus the tile/brush
//! rendering globals that sit beside them.
//!
//! `map_generator.create()` registers every map template and calls each one's
//! `init()`. The tactical tile templates (38 of them) build a tile-transition
//! descriptor there — `createTileTransition()` then `setBrush`/`setSocket`/
//! `setBlendIntoSockets` on it — and hand it to `Tactical.setTransitions`. None of
//! that can construct without these natives, so the object-graph load needs them.
//!
//! What's real vs. pending: the [`TileTransition`] descriptor faithfully records
//! its configuration, and `Tactical.setTransitions` stores it in a per-VM registry
//! keyed by tile-brush name. The *consumer* — the tactical renderer that draws the
//! blended tile borders from this data — is not built yet, so the registry is
//! written but not yet read. `doesBrushExist` likewise can't consult a real sprite
//! atlas yet, so it reports `false` (callers use it only to gate optional
//! decorative brushes, so a `false` simply skips them — never an error).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use squirrel::{ClassBuilder, Native, Retained, SQVM, SqArray, SqStrongRef, SqTable, Variadic};

/// One tile type's transition configuration — the payload behind a
/// `createTileTransition()` instance, and what `Tactical.setTransitions` caches.
///
/// Fields are recorded from script but not yet read: the tactical renderer that
/// blends tile borders from this data is a later subsystem.
#[derive(Clone)]
#[allow(dead_code)] // consumed by the not-yet-built tactical renderer
pub struct TileTransition {
	/// Socket name (`setSocket`) — which transition socket this tile plugs into.
	socket: Option<String>,
	/// Whether the tile blends into neighbouring sockets (`setBlendIntoSockets`);
	/// defaults true (only a few tiles, e.g. roads, turn it off).
	blend_into_sockets: bool,
	/// Per-direction border brushes (`setBrush(direction, name)`), in call order.
	brushes: Vec<(i64, String)>,
}

impl TileTransition {
	fn new() -> Self {
		TileTransition {
			socket: None,
			blend_into_sockets: true,
			brushes: Vec::new(),
		}
	}
}

/// Register the tile-transition class, the `createTileTransition` / `doesBrushExist`
/// root globals, and the `Tactical` table (currently just `setTransitions`).
pub(crate) fn install(vm: &mut SQVM) {
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

	// doesBrushExist(name) -> false until a real sprite atlas exists (callers only
	// gate optional decorative brushes on it, so false safely skips them).
	vm.register("doesBrushExist", |_name: &str| false);

	// Tactical.setTransitions(tileBrushName, transition): cache the descriptor for
	// the (future) renderer. The registry holds plain Rust data (no VM handles), so
	// it has no teardown ordering concerns; it lives for the VM's lifetime via the
	// closure capture.
	let registry: Rc<RefCell<HashMap<String, TileTransition>>> =
		Rc::new(RefCell::new(HashMap::new()));
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

	// Tactical.getCamera(): a headless camera that moves instantly. The AI waits until
	// the camera is centred on its unit or the action is on screen
	// (`agent.IsCameraReady`), so both always hold; it is never moving. `Level`,
	// `Zoom` and `IsFlipped` are plain fields scripts read back.
	let camera = {
		let noop = vm.make_closure(|Variadic(_a): Variadic<Retained>| {});
		let yes = vm.make_closure(|Variadic(_a): Variadic<Retained>| true);
		let no = vm.make_closure(|Variadic(_a): Variadic<Retained>| false);
		let zero = vm.make_closure(|Variadic(_a): Variadic<Retained>| 0i64);
		let empty = vm.make_closure(|vm: &SQVM, Variadic(_a): Variadic<Retained>| SqArray::new(vm).to_strong());
		let get_pos = vm.make_closure(|vm: &SQVM| -> Option<SqStrongRef> {
			SqTable::root(vm)
				.get::<SqStrongRef>("createVec")
				.and_then(|create| create.call_ret::<SqStrongRef, _>(vm, (0.0, 0.0)).ok().flatten())
		});
		let mut cam = SqTable::new(vm);
		cam.set("Level", 0i64);
		cam.set("Zoom", 1.0);
		cam.set("IsFlipped", false);
		for name in [
			"registerCallbacks", "unregisterCallbacks", "setLevelToHighestOnMap", "quake", "zoomBy",
			"zoomTo", "move", "moveToExactly", "climbBy", "update", "render", "updateEntityOverlays",
			"addEntityOverlay", "removeEntityOverlay", "removeEntityOverlays", "cancelMovement",
			"setPos", "moveTo", "jumpTo", "moveToTile", "moveToTileSlowly",
		] {
			cam.set(name, noop.clone());
		}
		cam.set("isCenteredOn", yes.clone());
		cam.set("isCenteredOnTile", yes.clone());
		cam.set("isInsideScreen", yes);
		cam.set("isMoving", no.clone());
		cam.set("isMovingToTile", no);
		cam.set("getPos", get_pos);
		cam.set("getBestLevelForTile", zero);
		cam.set("queryEntityOverlays", empty);
		cam.to_strong()
	};
	let get_camera = vm.make_closure(move || camera.clone());

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
	tactical.set("getCamera", get_camera);
	tactical.set("getWeather", get_weather);
	tactical.set("getShaker", get_shaker);
	tactical.set("getHighlighter", get_highlighter);
	tactical.set("setTransitions", set_transitions);
	tactical.set("setActive", set_active);
	tactical.set("isActive", is_active);
	tactical.set("setVisible", set_visible);
	tactical.set("isVisible", is_visible);
	SqTable::root(vm).set("Tactical", tactical);
}
