//! The RootState named-state stack, driven against synthetic state scripts: boot →
//! root.onInit → `add` children → each self-`show`s → sibling bus → `finish` →
//! the background-task hook. Uses an in-memory script source so the test is
//! self-contained (no game data).

use std::collections::HashMap;

use squirrel::{SQVM, SqArray, SqTable};
use squirrel_bb::{KeyInput, LoadedScript, ModuleSystem, MouseInput, RootState, ScriptSource, install_natives};

/// A trivial in-memory [`ScriptSource`] mapping path → source text.
struct MemSource(HashMap<String, String>);

impl ScriptSource for MemSource {
	fn load(&self, path: &str) -> Option<LoadedScript> {
		self.0
			.get(path)
			.map(|s| LoadedScript::Source(s.clone().into_bytes()))
	}
	fn list_prefix(&self, _prefix: &str) -> Option<Vec<String>> {
		None
	}
}

fn sources() -> MemSource {
	let mut m = HashMap::new();
	// Base: virtual hooks that log via ::log (getName is injected natively).
	m.insert(
		"scripts/states/state".into(),
		r#"
		this.state <- {
			m = {},
			function onInit()  { ::log.push(this.getName() + ":onInit"); }
			function onShow()  { ::log.push(this.getName() + ":onShow"); }
			function onHide()  { ::log.push(this.getName() + ":onHide"); }
			function onFinish(){ ::log.push(this.getName() + ":onFinish"); }
			function onSiblingSentMessage( _from, _m ) { ::log.push(this.getName() + ":sib:" + _from + ":" + _m); }
		};
		"#
		.into(),
	);
	// Root: sets the Root global (like vanilla) and adds two child states.
	m.insert(
		"scripts/root_state".into(),
		r#"
		this.root_state <- this.inherit("scripts/states/state", {
			function onInit() {
				::Root <- this;
				::log.push(this.getName() + ":onInit");
				this.add("MenuA", "scripts/states/menu_a");
				this.add("MenuB", "scripts/states/menu_b");
			}
		});
		"#
		.into(),
	);
	// MenuA: self-shows in onInit (the convention the engine relies on).
	m.insert(
		"scripts/states/menu_a".into(),
		r#"
		this.menu_a <- this.inherit("scripts/states/state", {
			function onInit() { ::log.push(this.getName() + ":onInit"); this.show(); }
		});
		"#
		.into(),
	);
	// MenuB: messages its siblings on init (MenuA should receive it).
	m.insert(
		"scripts/states/menu_b".into(),
		r#"
		this.menu_b <- this.inherit("scripts/states/state", {
			function onInit() { ::log.push(this.getName() + ":onInit"); this.sendMessageToSiblings("hi"); }
		});
		"#
		.into(),
	);
	MemSource(m)
}

#[test]
fn boots_root_adds_children_and_runs_the_sibling_bus() {
	let mut vm = SQVM::new(1 << 16);
	install_natives(&mut vm);
	let modules = ModuleSystem::new(sources());
	modules.install(&mut vm);
	let root_state = RootState::new(modules.instantiator());
	root_state.install(&mut vm);

	vm.run_script("::log <- []; ::bgFired <- false;", "setup")
		.expect("setup globals");

	root_state
		.boot(&vm, "scripts/root_state")
		.expect("boot the root state");

	// add() registered both children, in order.
	assert_eq!(root_state.state_names(), ["MenuA", "MenuB"]);
	assert!(root_state.root().is_some());
	assert!(root_state.state("MenuA").is_some());

	// The full lifecycle ran: root.onInit → add MenuA (onInit→self show) → add MenuB
	// (onInit → message siblings → MenuA receives it). getName works throughout
	// (the names in the log come from the injected getName).
	let log: Vec<String> = SqTable::root(&vm)
		.get::<SqArray>("log")
		.expect("::log")
		.to_vec::<String>();
	assert_eq!(
		log,
		[
			"RootState:onInit",
			"MenuA:onInit",
			"MenuA:onShow",
			"MenuB:onInit",
			"MenuA:sib:MenuB:hi",
		]
	);

	// finish() only flags the state, as in the original: it's still registered until
	// the next update pass, which runs its onFinish and removes it.
	root_state
		.state("MenuA")
		.unwrap()
		.call_method(&vm, "finish", ())
		.expect("finish MenuA");
	assert_eq!(root_state.state_names(), ["MenuA", "MenuB"]);
	root_state.update(&vm).expect("update pass");
	assert_eq!(root_state.state_names(), ["MenuB"]);
	assert!(root_state.state("MenuA").is_none());

	root_state.clear();
}

#[test]
fn background_task_hook() {
	let mut vm = SQVM::new(1 << 16);
	install_natives(&mut vm);
	let modules = ModuleSystem::new(sources());
	modules.install(&mut vm);
	let root_state = RootState::new(modules.instantiator());
	root_state.install(&mut vm);

	vm.run_script("::log <- []; ::bgFired <- false;", "setup")
		.expect("setup globals");
	root_state.boot(&vm, "scripts/root_state").expect("boot");

	// Scripts register a background callback and poll the suspended flag on Root.
	vm.run_script(
		r#"
		::Root.setBackgroundTaskCallback(function () { ::bgFired = true; });
		::wasSuspended <- ::Root.isBackgroundTaskSuspended();
		"#,
		"bg",
	)
	.expect("background hook calls");

	// The flag defaults false; the host can flip it; the callback is captured and
	// the host can fire it (the cooperative-yield model worldgen uses).
	assert_eq!(SqTable::root(&vm).get::<bool>("wasSuspended"), Some(false));
	root_state.set_suspended(true);
	vm.run_script(
		"::nowSuspended <- ::Root.isBackgroundTaskSuspended();",
		"bg2",
	)
	.expect("poll again");
	assert_eq!(SqTable::root(&vm).get::<bool>("nowSuspended"), Some(true));

	let callback = root_state
		.background_callback()
		.expect("callback registered");
	callback.call(&vm, ()).expect("fire background callback");
	assert_eq!(SqTable::root(&vm).get::<bool>("bgFired"), Some(true));

	root_state.clear();
}

/// The host runs the background task inline, suspended from the start, as the
/// original's main thread wants the VM back at once: a task past its time allotment
/// yields (`isAllottedTimeReached`). Outside the task the flag is as it was.
#[test]
fn background_task_runs_suspended() {
	let mut vm = SQVM::new(1 << 16);
	install_natives(&mut vm);
	let modules = ModuleSystem::new(sources());
	modules.install(&mut vm);
	let root_state = RootState::new(modules.instantiator());
	root_state.install(&mut vm);

	vm.run_script("::log <- []; ::inside <- null;", "setup").expect("setup globals");
	root_state.boot(&vm, "scripts/root_state").expect("boot");
	vm.run_script(
		"::Root.setBackgroundTaskCallback(function () { ::inside = ::Root.isBackgroundTaskSuspended(); });",
		"bg",
	)
	.expect("register the task");

	root_state.run_background(&vm).expect("run the task");
	assert_eq!(SqTable::root(&vm).get::<bool>("inside"), Some(true), "suspended while it runs");
	vm.run_script("::outside <- ::Root.isBackgroundTaskSuspended();", "after").expect("poll after");
	assert_eq!(SqTable::root(&vm).get::<bool>("outside"), Some(false), "as it was afterwards");

	root_state.clear();
}

/// A state added by the scripts starts hidden, as the original's (probed 2026-10-19:
/// `isVisible()` is false from `RootState.add` through its `onInit`, and pointer moves
/// don't reach it): it gets no input and runs no frames until it shows itself, which
/// vanilla's do at the end of their `init()`. Mods hooking `onMouseInput` rely on that
/// (Configurable Pause reads the player's party there).
#[test]
fn an_added_state_waits_hidden_until_it_shows() {
	let mut m = HashMap::new();
	m.insert(
		"scripts/states/state".into(),
		r#"
		this.state <- {
			m = {},
			function onInit() {}
			function onShow() { ::log.push(this.getName() + ":onShow"); }
			function onUpdate() { ::log.push(this.getName() + ":onUpdate"); }
			function onMouseInput( _m ) { ::log.push(this.getName() + ":mouse"); return false; }
			function onKeyInput( _k ) { ::log.push(this.getName() + ":key"); return false; }
		};
		"#
		.into(),
	);
	m.insert(
		"scripts/root_state".into(),
		r#"this.root_state <- this.inherit("scripts/states/state", { function onInit() { ::Root <- this; } });"#.into(),
	);
	// Its onInit doesn't show it: a later call does (as world_state's init()).
	m.insert(
		"scripts/states/late".into(),
		r#"this.late <- this.inherit("scripts/states/state", { function onInit() { ::log.push("Late:onInit"); } });"#.into(),
	);
	let mut vm = SQVM::new(1 << 16);
	install_natives(&mut vm);
	let modules = ModuleSystem::new(MemSource(m));
	modules.install(&mut vm);
	let root_state = RootState::new(modules.instantiator());
	root_state.install(&mut vm);
	vm.run_script("::log <- [];", "setup").expect("setup globals");
	root_state.boot(&vm, "scripts/root_state").expect("boot");

	let take = |vm: &SQVM| -> Vec<String> {
		let log = SqTable::root(vm).get::<SqArray>("log").expect("::log").to_vec::<String>();
		vm.run_script("::log.clear();", "clear").expect("clear the log");
		log
	};
	let input = |vm: &SQVM| {
		root_state.mouse_input(vm, MouseInput::moved(10, 20)).expect("mouse");
		root_state.key_input(vm, KeyInput::pressed(41)).expect("key");
	};

	vm.run_script(
		r#"::late <- ::RootState.add("Late", "scripts/states/late"); ::shownAtAdd <- ::late.isVisible();"#,
		"add",
	)
	.expect("add Late");
	assert_eq!(SqTable::root(&vm).get::<bool>("shownAtAdd"), Some(false), "hidden once added");
	// Added, not initialized: input reaches only the root.
	input(&vm);
	assert_eq!(take(&vm), ["RootState:mouse", "RootState:key"]);

	// Its onInit runs on the next update; still hidden, it runs no frames.
	root_state.update(&vm).expect("update");
	assert_eq!(take(&vm), ["Late:onInit", "RootState:onUpdate"]);
	input(&vm);
	assert_eq!(take(&vm), ["RootState:mouse", "RootState:key"]);

	// Shown: input goes to it first, and it updates.
	vm.run_script("::late.show();", "show").expect("show Late");
	input(&vm);
	root_state.update(&vm).expect("update");
	assert_eq!(
		take(&vm),
		["Late:onShow", "Late:mouse", "RootState:mouse", "Late:key", "RootState:key", "RootState:onUpdate", "Late:onUpdate"]
	);
	root_state.clear();
}
