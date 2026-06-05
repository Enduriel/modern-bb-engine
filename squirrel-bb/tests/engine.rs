//! The [`Engine`] host: composes the VM + all four services, boots a (synthetic)
//! state graph that connects to the UI bridge, and tears everything down on drop
//! with no manual per-service `clear()` — the regression guard for the teardown
//! drop-order contract.

use std::collections::HashMap;

use squirrel_bb::{Engine, LoadedScript, ScriptSource};

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
	m.insert(
		"scripts/states/state".into(),
		"this.state <- { m = {}, function onInit(){} function onShow(){} function onFinish(){} };"
			.into(),
	);
	m.insert(
		"scripts/root_state".into(),
		r#"this.root_state <- this.inherit("scripts/states/state", {
			function onInit() { ::Root <- this; this.add("Screen", "scripts/states/screen_state"); }
		});"#
			.into(),
	);
	// A state that connects to the UI bridge (creating a JSHandle native instance),
	// records SQ→JS calls, and self-shows.
	m.insert(
		"scripts/states/screen_state".into(),
		r#"this.screen_state <- this.inherit("scripts/states/state", {
			function onInit() {
				this.m.handle <- this.UI.connect("MyScreen", this);
				this.m.handle.asyncCall("setData", { x = 1 });
				this.show();
			}
			function onShow() { this.m.handle.asyncCall("show", true); }
		});"#
			.into(),
	);
	MemSource(m)
}

#[test]
fn engine_boots_a_graph_and_tears_down_cleanly() {
	let mut engine = Engine::new(sources());
	engine.vm_mut().set_print_handler(|_| {});
	engine.run_script("::log <- [];", "setup").expect("setup");

	engine.boot("scripts/root_state").expect("boot");

	// State stack + module system worked through the host.
	assert_eq!(engine.root_state().state_names(), ["Screen"]);
	assert!(engine.root_state().state("Screen").is_some());

	// The UI bridge recorded the screen's SQ→JS calls (via a real JSHandle instance).
	let methods = engine.ui().methods_for("MyScreen");
	assert!(methods.contains(&"setData".to_string()));
	assert!(
		methods.contains(&"show".to_string()),
		"onShow ran via self-show"
	);
	assert_eq!(engine.ui().call_count(), 2);

	// `engine` drops at end of scope → clears all services then closes the VM. If the
	// drop-before-close contract were broken this would crash (it must not).
}

#[test]
fn engine_accessors_and_run_script() {
	let engine = Engine::new(sources());
	engine
		.run_script("::answer <- 6 * 7;", "calc")
		.expect("run");
	// vm() accessor reaches the result.
	let answer = squirrel::SqTable::root(engine.vm()).get::<i64>("answer");
	assert_eq!(answer, Some(42));
	assert!(engine.root_state().root().is_none(), "not booted yet");
}
