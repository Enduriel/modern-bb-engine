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
		local gt = this.getroottable();
		if (!("counted" in gt)) { gt.counted <- 0; }
		gt.counted = gt.counted + 1;
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

/// Each included file runs in a scope of its own (probed in the original): a fresh
/// table as `this`, so its definitions stay there and a class named like a global
/// (vanilla's `skills/actives/split`) doesn't replace it; globals are still read
/// through the root fallback, and a base class is adopted from the file's scope.
#[test]
fn include_runs_each_file_in_its_own_scope() {
	let (vm, modules) = vm_with(&[
		(
			"scripts/skills/actives/split",
			r#"
			::__sawScope <- this != this.getroottable();
			::__readsGlobals <- typeof this.getroottable == "function" && this.Shared == 7;
			this.split <- { m = { Hits = 3 } };
			function helper() { return this; }
			::__helperThisIsScope <- helper() != this.getroottable();
			"#,
		),
	]);
	vm.run_script("::Shared <- 7; ::__stdSplit <- ::split;", "setup").expect("setup");
	modules.include(&vm, "scripts/skills/actives/split").expect("include");
	vm.run_script(
		r#"
		assert(::__sawScope);
		assert(::__readsGlobals);
		assert(::__helperThisIsScope);
		assert(::split == ::__stdSplit);               // the stdlib split survives
		assert(!("helper" in this.getroottable()));    // file-scope functions stay put
		local s = this.new("scripts/skills/actives/split"); // adopted from the scope
		assert(s.m.Hits == 3);
		"#,
		"scope_test",
	)
	.expect("the file ran in its own scope");
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
		// Like the original: only an existing script, by its exact canonical path.
		assert(IO.scriptHashByFilename("scripts/nope") == null);
		assert(IO.scriptHashByFilename("scripts/skills/bash.nut") == null);
		assert(IO.scriptHashByFilename("") == null);

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

// A three-level chain mirroring entity → actor → player, for the layout facts probed
// in the ORIGINAL engine (2026-10-13).
const L_ENTITY: &str = r#"
this.entity <- {
	m = { ID = 0, Name = "" },
	function getName() { return this.m.Name; }
	function whoAmI() { return this; }
};
"#;
const L_ACTOR: &str = r#"
this.actor <- this.inherit("scripts/entity/tactical/entity", {
	m = { Hitpoints = 10, Stored = null },
	function create() { this.m.Name = "actor"; }
	function onInit() { this.m.Stored = this; }      // like setActor(this)
	function getHitpoints() { return this.m.Hitpoints; }
});
"#;
const L_PLAYER: &str = r#"
this.player <- this.inherit("scripts/entity/tactical/actor", {
	m = { Mood = 3 },
	function create() { this.actor.create(); this.m.Name = "player"; }
	function onInit() { this.actor.onInit(); }
	function getMoodState() { return this.m.Mood; }    // derived-only
	function getName() { return "P:" + this.actor.getName(); }
});
"#;

fn three_levels() -> (SQVM, ModuleSystem<MemSource>) {
	vm_with(&[
		("scripts/entity/tactical/entity", L_ENTITY),
		("scripts/entity/tactical/actor", L_ACTOR),
		("scripts/entity/tactical/player", L_PLAYER),
	])
}

#[test]
fn super_calls_run_with_this_as_the_leaf() {
	let (vm, _m) = three_levels();
	vm.run_script(
		r#"
		local p = this.new("scripts/entity/tactical/player");
		p.onInit();
		// actor.onInit stored `this` during a super-call: it must be the leaf …
		assert(p.m.Stored == p);
		// … so derived-only methods resolve through it (the getMoodState case).
		assert(p.m.Stored.getMoodState() == 3);
		// Methods found on a super level are bound to the leaf: .call(other) is ignored.
		local other = this.new("scripts/entity/tactical/player");
		assert(p.actor.whoAmI() == p);
		assert(p.actor.whoAmI.call(other) == p);
		// Super dispatch still picks the ancestor's version.
		assert(p.getName() == "P:player");
		"#,
		"super_this",
	)
	.expect("super-calls keep this = leaf");
}

#[test]
fn instance_levels_and_member_tables_match_the_original_layout() {
	let (vm, _m) = three_levels();
	vm.run_script(
		r#"
		local p = this.new("scripts/entity/tactical/player");
		local q = this.new("scripts/entity/tactical/player");
		local a = p.actor, e = a.entity;
		// Per-instance level tables, chained by delegate, base has none.
		assert(p.getdelegate() == a && a.getdelegate() == e && e.getdelegate() == null);
		assert(p.actor != q.actor);
		// Each level holds only its own methods.
		assert(p.rawin("getMoodState") && !a.rawin("getMoodState") && !p.rawin("getHitpoints"));
		// Each level has its own m with only its declared fields, delegate-chained.
		assert(p.m.rawin("Mood") && !p.m.rawin("Hitpoints") && !p.m.rawin("ID"));
		assert(a.m.rawin("Hitpoints") && e.m.rawin("ID"));
		assert(p.m.getdelegate() == a.m && a.m.getdelegate() == e.m);
		// Reads fall through; writes land in the level that owns the field.
		p.m.ID = 42;
		assert(!p.m.rawin("ID") && e.m.rawget("ID") == 42 && a.m.ID == 42);
		// Instances never share member state.
		assert(q.m.ID == 0);
		// Metadata: ClassName per level, SuperName on derived levels.
		assert(p.ClassName == "player" && p.SuperName == "actor" && a.SuperName == "entity");
		assert(e.ClassName == "entity" && !e.rawin("SuperName"));
		"#,
		"layout",
	)
	.expect("layout matches the original engine");
}

/// Each class gets its own copy of its ancestors' levels at `inherit` (probed in the
/// original, 2026-10-14): the class table has no delegate, its super slot holds a linked
/// copy (each level delegating to the next, `m` to the parent's `m`), and a write
/// through one class's ancestor levels (as Modern Hooks' `q.m.X = v` does) reaches that
/// class's instances only.
#[test]
fn each_class_has_its_own_copy_of_its_ancestors() {
	let (vm, _m) = vm_with(&[
		("scripts/items/item", "this.item <- { m = { BaseItemScript = null, ID = \"\" }, function create() {} }; ::item <- this.item;"),
		("scripts/items/weapon", "this.weapon <- this.inherit(\"scripts/items/item\", { m = { Damage = 1 }, function create() { this.item.create(); } }); ::weapon <- this.weapon;"),
		("scripts/items/mace", "this.mace <- this.inherit(\"scripts/items/weapon\", { m = {}, function create() { this.weapon.create(); } }); ::mace <- this.mace;"),
		("scripts/items/named_mace", "this.named_mace <- this.inherit(\"scripts/items/weapon\", { m = {}, function create() { this.weapon.create(); } }); ::named_mace <- this.named_mace;"),
	]);
	vm.run_script(
		r#"
		::new("scripts/items/mace");
		::new("scripts/items/named_mace");
		local named = ::named_mace, mace = ::mace;
		assert(named.getdelegate() == null && named.m.getdelegate() == null);
		local w = named.weapon, i = w.item;
		assert(w.getdelegate() == i && w.m.getdelegate() == i.m && i.getdelegate() == null);
		assert(w != mace.weapon && i != mace.weapon.item && i != ::item);
		// Where Modern Hooks' setM writes BaseItemScript for named_mace: its own copy.
		i.m.BaseItemScript = "scripts/items/mace";
		assert(::new("scripts/items/named_mace").m.BaseItemScript == "scripts/items/mace");
		assert(::new("scripts/items/mace").m.BaseItemScript == null);
		assert(::item.m.BaseItemScript == null);
		"#,
		"class_copies",
	)
	.expect("each class has its own copy of its ancestors");
}

#[test]
fn instances_carry_the_originals_extra_slots() {
	// What the original puts on every instance besides its classes' slots (parity probe
	// `menu/class_layout`): a release-hook userdata on the leaf; on the base level,
	// `InstanceName` ("<class>@<n>", n counting the class's instances) and the
	// `self`/`base` natives, bound to the instance.
	let (vm, _ms) = vm_with(&[
		("scripts/base_thing", "this.base_thing <- { m = {}, function create() {} };"),
		("scripts/thing", "this.thing <- this.inherit(\"scripts/base_thing\", { m = {}, function create() { this.base_thing.create(); } });"),
	]);
	vm.run_script(
		r#"
		local a = ::new("scripts/thing"), b = ::new("scripts/thing");
		assert(typeof a._release_hook_DO_NOT_delete_it_ == "userdata");
		assert(a._release_hook_DO_NOT_delete_it_ != b._release_hook_DO_NOT_delete_it_);
		assert(a.base_thing.rawin("InstanceName") && !a.rawin("InstanceName"));
		assert(a.InstanceName == "thing@1" && b.InstanceName == "thing@2");
		assert(a.base_thing.rawin("self") && a.base_thing.rawin("base") && !a.rawin("self"));
		assert(a["self"]() == a);
		assert(a["base"]() == a.base_thing);
		assert(a["self"] != b["self"]);
		"#,
		"extra_slots",
	)
	.expect("the original's extra slots");
}

/// Lazy class levels (the default) look to scripts exactly as plain ones do: the same
/// script, run once over each, sees the same slots in the same iteration order, the same
/// bindings and identities, through super slots, writes to the levels, a class patched
/// after instances exist (they keep what they were made from), and `m` written through.
#[test]
fn lazy_levels_look_like_plain_ones() {
	const SCRIPT: &str = r#"
	local out = [];
	local function show(label, t) {
		local s = label + ":";
		foreach (k, v in t) s += " " + k + "=" + (typeof v == "function" ? "fn:" + v.getinfos().name : typeof v == "table" || typeof v == "userdata" ? typeof v : v);
		out.push(s + " #" + t.len());
	}
	local p = ::new("scripts/entity/tactical/player"), q = ::new("scripts/entity/tactical/player");
	out.push("identity " + (p.getHitpoints == p.getHitpoints) + " " + (p.getHitpoints == q.getHitpoints) + " " + (p.actor.getHitpoints == p.getHitpoints)
		+ " " + (p.create == q.create) + " " + (p.actor.whoAmI.call(q) == p) + " " + p.getName());
	out.push("rawin " + p.actor.rawin("getHitpoints") + " " + p.actor.rawin("getMoodState") + " " + p.actor.entity.rawin("whoAmI") + " " + p.actor.entity.rawin("InstanceName"));
	// Writes to an ancestor level, a new slot on it, and m written through.
	p.actor.extra <- function() { return "extra"; };
	p.actor.entity.getName = function() { return "patched on the instance"; };
	p.m.ID = 7;
	// The class patched after p and q exist: they keep what they were made from.
	::player_class.actor.getHitpoints = function() { return -1; };
	::player_class.actor.added <- 1;
	::player_class.actor.entity.whoAmI = function() { return "patched class"; };
	local r = ::new("scripts/entity/tactical/player");
	// The class's members patched after p, q and r exist; a copy of q's untouched ones.
	::player_class.actor.m.Hitpoints = 99;
	::player_class.actor.m.NewField <- 3;
	out.push("members " + q.m.Hitpoints + " " + ("NewField" in q.actor.m) + " " + ("NewField" in ::new("scripts/entity/tactical/player").actor.m) + " " + ::new("scripts/entity/tactical/player").m.Hitpoints);
	show("mclone", clone q.actor.m);
	out.push("patched " + p.getHitpoints() + " " + ("added" in p.actor) + " " + r.getHitpoints() + " " + ("added" in r.actor) + " " + (p.actor.entity.whoAmI() == p) + " " + r.actor.entity.whoAmI());
	out.push("names " + p.getName() + " " + q.getName() + " " + p.actor.extra() + " " + p.m.ID + " " + q.m.ID);
	foreach (i, t in [p, p.actor, p.actor.entity, q.actor, r.actor, r.actor.entity, ::player_class.actor, ::player_class.actor.entity, ::actor_class, p.m, p.actor.m, p.actor.entity.m])
		show("level" + i, t);
	// After iterating (the levels are plain now): reads, identity and delete still agree.
	local f = p.actor.getHitpoints;
	out.push("after " + (f == p.getHitpoints) + " " + p.getHitpoints() + " " + delete p.actor.extra + " " + ("extra" in p.actor) + " " + clone q.actor.entity == null);
	show("clone", clone r.actor.entity);
	q.actor.entity.rawdelete("whoAmI");
	show("deleted", q.actor.entity);
	r.actor.clear();
	show("cleared", r.actor);
	::__out <- out.reduce(@(a, b) a + "\n" + b);
	"#;
	let run = |lazy: bool| {
		let (vm, modules) = vm_with(&[
			("scripts/entity/tactical/entity", L_ENTITY),
			("scripts/entity/tactical/actor", "this.actor <- this.inherit(\"scripts/entity/tactical/entity\", { m = { Hitpoints = 10, Stored = null }, function create() { this.m.Name = \"actor\"; } function getHitpoints() { return this.m.Hitpoints; } }); ::actor_class <- this.actor;"),
			("scripts/entity/tactical/player", &format!("{L_PLAYER} ::player_class <- this.player;")),
		]);
		modules.set_lazy_levels(lazy);
		modules.set_lazy_members(lazy);
		vm.run_script(SCRIPT, "compare").expect("the script runs");
		let out = squirrel::SqTable::root(&vm).get::<String>("__out").expect("its output");
		vm.collect_garbage();
		out
	};
	let plain = run(false);
	let lazy = run(true);
	assert_eq!(lazy, plain, "lazy levels:\n{lazy}\n\nplain levels:\n{plain}");
	assert!(plain.contains("patched 10 false -1 true true patched class"), "{plain}");
	assert!(plain.contains("members 10 false true 99"), "{plain}");
}
