//! End-to-end tests of the BB module system against representative vanilla
//! patterns (a base + derived skill, a multi-level entity chain with super-calls,
//! run-once `include`, and the `IO` hash registry). Each test drives the system
//! the way scripts do — through `this.new` / `this.inherit` / `isKindOf` — and
//! asserts script-side, so a failure surfaces as a runtime error.

use std::collections::HashMap;

use squirrel::SQVM;
use squirrel_bb::ModuleSystem;

/// An in-memory [`ScriptSource`](squirrel_bb::ScriptSource): path → source bytes.
struct MemSource(HashMap<String, Vec<u8>>);

impl MemSource {
	fn new(files: &[(&str, &str)]) -> Self {
		MemSource(
			files
				.iter()
				.map(|(p, s)| ((*p).to_string(), s.as_bytes().to_vec()))
				.collect(),
		)
	}
}

impl squirrel_bb::ScriptSource for MemSource {
	fn load(&self, path: &str) -> Option<squirrel_bb::LoadedScript> {
		self.0
			.get(path)
			.cloned()
			.map(squirrel_bb::LoadedScript::Source)
	}

	fn list_prefix(&self, prefix: &str) -> Option<Vec<String>> {
		// Match at a directory boundary: ensure the prefix ends with '/'.
		let dir = if prefix.is_empty() || prefix.ends_with('/') {
			prefix.to_string()
		} else {
			format!("{prefix}/")
		};
		let mut paths: Vec<String> = self
			.0
			.keys()
			.filter(|key| key.starts_with(&dir))
			.cloned()
			.collect();
		if paths.is_empty() {
			return None; // no such directory (lets the DLC sweep see null)
		}
		paths.sort();
		Some(paths)
	}
}

/// Build a VM with the module system installed over `files`.
fn vm_with(files: &[(&str, &str)]) -> (SQVM, ModuleSystem<MemSource>) {
	let mut vm = SQVM::new(4096);
	let modules = ModuleSystem::new(MemSource::new(files));
	modules.install(&mut vm);
	(vm, modules)
}

// A base class registered as a plain literal (no `create`; its `m` holds the
// defaults), plus a derived class via `inherit` whose `create` overrides some of
// them. Mirrors scripts/skills/skill.nut + scripts/skills/actives/bash.nut.
const SKILL: &str = r#"
this.skill <- {
	m = { ID = "", Name = "", HitChance = 0 },
	function getName() { return this.m.Name; }
	function getHitChance() { return this.m.HitChance; }
};
"#;

const BASH: &str = r#"
this.bash <- this.inherit("scripts/skills/skill", {
	m = {},
	function create() {
		this.m.ID = "bash";
		this.m.Name = "Bash";
		// HitChance is left at the inherited default.
	}
	function getDescription() { return "bash desc"; }
});
"#;

#[test]
fn derived_class_merges_defaults_and_overrides_in_create() {
	let (vm, _m) = vm_with(&[
		("scripts/skills/skill", SKILL),
		("scripts/skills/bash", BASH),
	]);

	vm.run_script(
		r#"
		local b = this.new("scripts/skills/bash");
		// create() overrides:
		assert(b.m.ID == "bash");
		assert(b.m.Name == "Bash");
		// inherited m default (never touched by create):
		assert(b.m.HitChance == 0);
		// inherited methods see the shared member table:
		assert(b.getName() == "Bash");
		assert(b.getHitChance() == 0);
		// own method:
		assert(b.getDescription() == "bash desc");
		"#,
		"skill_test",
	)
	.expect("derived skill behaves correctly");
}

#[test]
fn is_kind_of_matches_self_and_ancestors() {
	let (vm, _m) = vm_with(&[
		("scripts/skills/skill", SKILL),
		("scripts/skills/bash", BASH),
	]);

	vm.run_script(
		r#"
		local b = this.new("scripts/skills/bash");
		assert(isKindOf(b, "bash"));   // its own class
		assert(isKindOf(b, "skill"));  // an ancestor
		assert(!isKindOf(b, "actor")); // unrelated
		"#,
		"iskindof_test",
	)
	.expect("isKindOf walks the chain");
}

// A three-level chain whose every level contributes to construction via
// `this.<parent>.create()` super-calls, all sharing one member table.
const ENTITY: &str = r#"
this.entity <- {
	m = { HP = 0, Name = "" },
	function create() { this.m.HP = 10; }
	function getHP() { return this.m.HP; }
};
"#;

const ACTOR: &str = r#"
this.actor <- this.inherit("scripts/entity", {
	m = {},
	function create() {
		this.entity.create();      // super-call
		this.m.Name = "actor";
	}
});
"#;

const NOMAD: &str = r#"
this.nomad <- this.inherit("scripts/actor", {
	m = {},
	function create() {
		this.actor.create();       // super-call (which super-calls entity)
		this.m.HP = this.m.HP + 5; // builds on the inherited HP
	}
});
"#;

#[test]
fn multi_level_chain_shares_member_table_through_super_calls() {
	let (vm, _m) = vm_with(&[
		("scripts/entity", ENTITY),
		("scripts/actor", ACTOR),
		("scripts/nomad", NOMAD),
	]);

	vm.run_script(
		r#"
		local n = this.new("scripts/nomad");
		// entity.create() set HP=10 (shared m), nomad.create() then added 5:
		assert(n.m.HP == 15);
		// actor.create() set the name on the same shared m:
		assert(n.m.Name == "actor");
		// inherited method reads the shared m too:
		assert(n.getHP() == 15);
		// type checks across all three levels:
		assert(isKindOf(n, "nomad"));
		assert(isKindOf(n, "actor"));
		assert(isKindOf(n, "entity"));
		"#,
		"chain_test",
	)
	.expect("the super-call chain shares one member table");
}

#[test]
fn instances_do_not_share_mutable_member_state() {
	let (vm, _m) = vm_with(&[
		("scripts/entity", ENTITY),
		("scripts/actor", ACTOR),
		("scripts/nomad", NOMAD),
	]);

	vm.run_script(
		r#"
		local a = this.new("scripts/nomad");
		local b = this.new("scripts/nomad");
		a.m.HP = 100;
		assert(a.m.HP == 100);
		assert(b.m.HP == 15, "second instance is independent");
		"#,
		"independence_test",
	)
	.expect("each instance gets its own member table");
}

#[test]
fn include_runs_each_script_at_most_once() {
	let (vm, modules) = vm_with(&[(
		"scripts/counted",
		r#"
		if (!("counted" in this)) { this.counted <- 0; }
		this.counted = this.counted + 1;
		"#,
	)]);

	modules
		.include(&vm, "scripts/counted")
		.expect("first include");
	modules
		.include(&vm, "scripts/counted")
		.expect("second include is a no-op");
	// Even a `.nut`-suffixed request resolves to the same already-loaded script.
	modules
		.include(&vm, "scripts/counted.nut")
		.expect("normalized no-op");

	vm.run_script(r#"assert(this.counted == 1);"#, "once_test")
		.expect("the script ran exactly once");
}

#[test]
fn io_registry_round_trips_and_matches_class_name_hash() {
	let (vm, _m) = vm_with(&[
		("scripts/skills/skill", SKILL),
		("scripts/skills/bash", BASH),
	]);

	vm.run_script(
		r#"
		local h = IO.scriptHashByFilename("scripts/skills/bash");
		assert(IO.scriptFilenameByHash(h) == "scripts/skills/bash");
		assert(IO.scriptFilenameByHash(999999) == null); // unknown hash -> null

		// The instance carries that same ClassNameHash.
		local b = this.new("scripts/skills/bash");
		assert(b.ClassNameHash == h);
		"#,
		"io_test",
	)
	.expect("IO registry round-trips with ClassNameHash");
}

#[test]
fn module_system_and_native_globals_coexist() {
	// Both subsystems install into the same root table; their slots are disjoint
	// and a module-loaded class can use the native value classes.
	let mut vm = SQVM::new(4096);
	let modules = ModuleSystem::new(MemSource::new(&[(
		"scripts/thing",
		r##"
		this.thing <- {
			m = {},
			function create() { this.m.Tint <- createColor("#ff8000"); }
			function tintR() { return this.m.Tint.R; }
		};
		"##,
	)]));
	modules.install(&mut vm);
	squirrel_bb::install_natives(&mut vm);

	vm.run_script(
		r#"
		local t = this.new("scripts/thing");
		assert(t.m.Tint instanceof color);   // native class, from a module-loaded script
		assert(t.tintR() == 255);            // 0xff8000 -> R = 255
		"#,
		"coexist",
	)
	.expect("module system + native globals work together");
}

#[test]
fn io_enumerate_files_lists_recursively_and_nulls_missing_dirs() {
	let (vm, _m) = vm_with(&[
		("scripts/config/global", "::loaded_global <- true;"),
		("scripts/config/world", "::loaded_world <- true;"),
		("scripts/entity/actor", "::loaded_actor <- true;"),
	]);

	vm.run_script(
		r#"
		// directory listing, lexicographically sorted
		local config = IO.enumerateFiles("scripts/config/");
		assert(config.len() == 2);
		assert(config[0] == "scripts/config/global");
		assert(config[1] == "scripts/config/world");

		// recursive over the whole tree
		assert(IO.enumerateFiles("scripts/").len() == 3);

		// a missing directory yields null (preload's DLC sweep relies on this)
		assert(IO.enumerateFiles("scripts/dlc/") == null);

		// the returned paths are include-able
		foreach (f in config) this.include(f);
		assert(loaded_global && loaded_world);
		"#,
		"enumerate",
	)
	.expect("IO.enumerateFiles lists, sorts, recurses, and nulls missing dirs");
}

#[test]
fn mini_preload_exercises_natives_module_system_and_io_together() {
	// A small end-to-end slice: a config file that uses boot-critical natives
	// (createColor + World.getTime) at load time, plus a base+derived class, loaded
	// via an enumerate+include sweep and then instantiated.
	let mut vm = SQVM::new(8192);
	let modules = ModuleSystem::new(MemSource::new(&[
		(
			"scripts/config/world_assets",
			r##"
			this.Const.World <- {
				FoodPerDay = 1.0 / this.World.getTime().SecondsPerDay,
				NeutralColor = this.createColor("#808080"),
			};
			"##,
		),
		(
			"scripts/skills/skill",
			r#"this.skill <- { m = { Name = "", Fatigue = 0 } };"#,
		),
		(
			"scripts/skills/actives/bash",
			r#"
			this.bash <- this.inherit("scripts/skills/skill", {
				m = {},
				function create() {
					this.m.Name = "Bash";
					this.m.Fatigue = 15;
					this.logInfo("bash created");
				}
			});
			"#,
		),
	]));
	modules.install(&mut vm);
	squirrel_bb::install_natives(&mut vm);

	// preload seeds Const, then sweeps + includes the config files.
	vm.run_script("::Const <- {};", "const_seed")
		.expect("seed Const");
	vm.run_script(
		r#"
		foreach (f in IO.enumerateFiles("scripts/config/")) this.include(f);
		// createColor + World.getTime ran at config-load time:
		assert(Const.World.FoodPerDay > 0.0);
		assert(Const.World.NeutralColor instanceof color);
		assert(Const.World.NeutralColor.R == 128);   // 0x80
		"#,
		"load_config",
	)
	.expect("config loads using natives at load time");

	// instantiate a derived class (loads skill + bash on demand via inherit/new)
	vm.run_script(
		r#"
		local b = this.new("scripts/skills/actives/bash");
		assert(b.m.Name == "Bash");
		assert(b.m.Fatigue == 15);   // set by create
		assert(isKindOf(b, "skill"));
		assert(isKindOf(b, "bash"));
		"#,
		"instantiate",
	)
	.expect("derived class instantiates with merged defaults");
}

#[test]
fn missing_script_is_reported() {
	let (vm, modules) = vm_with(&[]);
	let err = modules
		.include(&vm, "scripts/does/not/exist")
		.expect_err("missing script should error");
	assert!(
		matches!(err, squirrel_bb::ScriptError::NotFound(_)),
		"got {err:?}"
	);
}
