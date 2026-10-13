//! [`Engine`]: the host object that owns the VM and every stateful service, and
//! tears them down in the correct order.
//!
//! The four host services ([`ModuleSystem`], [`TimeService`], [`UiBridge`],
//! [`RootState`]) each hold [`SqStrongRef`](squirrel::SqStrongRef)s into the VM that
//! MUST be released before `sq_close` — and merely dropping a service handle is not
//! enough, because VM-resident closures keep its state alive until `sq_close` (which
//! is then too late: releasing during teardown is a use-after-free). So each service
//! has a `clear()`, and forgetting to call all four before closing the VM is an easy,
//! crashing mistake.
//!
//! `Engine` removes that footgun: it owns all of them, and its [`Drop`] `clear()`s
//! every service *while the VM is still fully alive*, then lets the `vm` field (the
//! last-declared field) drop last for a clean `sq_close`.

use squirrel::{SQVM, SqStrongRef};

use crate::error::ScriptError;
use crate::module::ModuleSystem;
use crate::native::{NativeConfig, TacticalEntities, TimeService, UiBridge, WorldEntities, WorldGrid};
use crate::source::ScriptSource;
use crate::state_stack::RootState;

/// The engine host: the VM plus all stateful services, with correct teardown.
///
/// Field order is load-bearing: Rust drops fields top-to-bottom, so `vm` is declared
/// **last** (dropped last). The [`Drop`] impl `clear()`s the services first (while
/// the VM lives), so no `SqStrongRef` is ever released into a closing VM.
pub struct Engine<S: ScriptSource> {
	root_state: RootState,
	world_grid: WorldGrid,
	world_entities: WorldEntities,
	tactical_entities: TacticalEntities,
	ui: UiBridge,
	time: TimeService,
	modules: ModuleSystem<S>,
	vm: SQVM,
}

impl<S: ScriptSource + 'static> Engine<S> {
	/// Build an engine over `source` with the default native config.
	pub fn new(source: S) -> Self {
		Self::with_config(source, &NativeConfig::default())
	}

	/// Build an engine over `source`, installing the stateless natives from `config`
	/// and all four stateful services. Stack size is the BB-typical 64K.
	pub fn with_config(source: S, config: &NativeConfig) -> Self {
		let mut vm = SQVM::new(1 << 16);
		config.install(&mut vm);

		let modules = ModuleSystem::new(source);
		modules.install(&mut vm);
		let time = TimeService::new();
		time.install(&mut vm);
		let ui = UiBridge::new();
		ui.install(&mut vm);
		// After the `World` table exists (config.install), add the grid natives.
		let world_grid = WorldGrid::new();
		world_grid.install(&mut vm);
		// After the grid natives (entity `getTile` delegates to `World.getTile`).
		let world_entities = WorldEntities::new(modules.instantiator());
		world_entities.install(&mut vm);
		// After the World table exists: the roster/tactical-unit host (provides
		// World.getRoster/getPlayerRoster/createRoster, replacing the bare stubs).
		let tactical_entities = TacticalEntities::new(modules.instantiator());
		tactical_entities.install(&mut vm);
		let root_state = RootState::new(modules.instantiator());
		root_state.install(&mut vm);

		Engine {
			root_state,
			world_grid,
			world_entities,
			tactical_entities,
			ui,
			time,
			modules,
			vm,
		}
	}

	// ---- accessors ----

	/// The VM (for reads / `&self` ops like `run_script`).
	pub fn vm(&self) -> &SQVM {
		&self.vm
	}
	/// The VM mutably (for the rare `&mut` op, e.g. setting handlers before boot).
	pub fn vm_mut(&mut self) -> &mut SQVM {
		&mut self.vm
	}
	pub fn modules(&self) -> &ModuleSystem<S> {
		&self.modules
	}
	pub fn time(&self) -> &TimeService {
		&self.time
	}
	pub fn ui(&self) -> &UiBridge {
		&self.ui
	}
	pub fn root_state(&self) -> &RootState {
		&self.root_state
	}
	pub fn world_grid(&self) -> &WorldGrid {
		&self.world_grid
	}
	pub fn world_entities(&self) -> &WorldEntities {
		&self.world_entities
	}
	pub fn tactical_entities(&self) -> &TacticalEntities {
		&self.tactical_entities
	}

	// ---- convenience ----

	/// Run a script string in the root scope.
	pub fn run_script(&self, code: &str, name: &str) -> Result<(), String> {
		self.vm.run_script(code, name)
	}

	/// `this.include(path)` — load+run a script once (e.g. `"scripts/preload"`).
	pub fn include(&self, path: &str) -> Result<(), ScriptError> {
		self.modules.include(&self.vm, path)
	}

	/// Boot the state stack at `root_path` (normally `"scripts/root_state"`).
	pub fn boot(&self, root_path: &str) -> Result<SqStrongRef, String> {
		self.root_state.boot(&self.vm, root_path)
	}

	/// Advance the host-driven clocks by `real_delta_ms` (fires due Real/Virtual
	/// scheduled events).
	pub fn tick(&self, real_delta_ms: f64) {
		self.time.tick(&self.vm, real_delta_ms);
	}

	/// Install logging to a custom sink (defaults to stderr/stdout otherwise).
	pub fn set_log_handler(&mut self, handler: impl Fn(crate::native::LogLevel, &str) + 'static) {
		crate::native::install_logging(&mut self.vm, handler);
	}
}

impl<S: ScriptSource> Drop for Engine<S> {
	fn drop(&mut self) {
		// Release every service's SqStrongRefs while the VM is still alive; the `vm`
		// field then drops last (clean `sq_close`).
		self.root_state.clear();
		self.tactical_entities.clear();
		self.world_entities.clear();
		self.world_grid.clear();
		self.ui.clear();
		self.time.clear();
		self.modules.clear();
	}
}
