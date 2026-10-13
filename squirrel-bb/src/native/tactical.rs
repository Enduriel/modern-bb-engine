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

use squirrel::{ClassBuilder, Native, SQVM, SqTable};

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

	// setActive(bool): world_state toggles tactical mode; a no-op headless (no combat
	// renderer/state). isActive reports false (we're never in tactical combat here).
	let set_active = vm.make_closure(|_active: bool| {});
	let is_active = vm.make_closure(|| false);

	let mut tactical = SqTable::new(vm);
	tactical.set("setTransitions", set_transitions);
	tactical.set("setActive", set_active);
	tactical.set("isActive", is_active);
	SqTable::root(vm).set("Tactical", tactical);
}
