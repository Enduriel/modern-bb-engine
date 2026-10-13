//! Tests for the native utility batch: the `Math` table, `String` + helpers,
//! logging, platform stubs, and that the Squirrel stdlib (string/math) is wired.

use std::cell::RefCell;
use std::rc::Rc;

use squirrel::SQVM;
use squirrel_bb::{LogLevel, NativeConfig, install_logging, install_natives};

fn vm() -> SQVM {
	let mut vm = SQVM::new(4096);
	install_natives(&mut vm);
	vm
}

#[test]
fn math_min_max_and_float_variants() {
	vm().run_script(
		r#"
			// integer variants truncate args then compare, returning an integer
			assert(Math.max(3, 7) == 7);
			assert(Math.min(3, 7) == 3);
			assert(Math.min(3, 2.9) == 2);          // 2.9 -> 2, min(3,2) == 2
			assert(typeof Math.max(1, 2) == "integer");
			// float variants keep floats
			assert(Math.maxf(1.5, 2.25) == 2.25);
			assert(Math.minf(1.5, 2.25) == 1.5);
			assert(typeof Math.maxf(1.0, 2.0) == "float");
			"#,
		"math_minmax",
	)
	.expect("Math min/max variants");
}

#[test]
fn math_abs_is_polymorphic() {
	vm().run_script(
		r#"
			assert(Math.abs(-5) == 5);
			assert(typeof Math.abs(-5) == "integer");   // int in -> int out
			assert(Math.abs(-2.5) == 2.5);
			assert(typeof Math.abs(-2.5) == "float");   // float in -> float out
			assert(Math.absf(-2.5) == 2.5);
			"#,
		"math_abs",
	)
	.expect("Math.abs polymorphism");
}

#[test]
fn math_rounding_returns_floats_and_pow_returns_float() {
	vm().run_script(
		r#"
			assert(Math.ceil(2.1) == 3 && typeof Math.ceil(2.1) == "float");
			assert(Math.floor(2.9) == 2 && typeof Math.floor(2.9) == "float");
				assert(typeof Math.round(2.5) == "float");
				local _a = [10, 20, 30];
				assert(_a[Math.floor(1.7)] == 20);
			assert(Math.round(2.5) == 3);               // half away from zero
			assert(Math.round(2.4) == 2);
			assert(Math.pow(2.0, 10.0) == 1024.0);
			assert(typeof Math.pow(2.0, 3.0) == "float");
			"#,
		"math_round",
	)
	.expect("Math rounding/pow types");
}

#[test]
fn math_get_angle_to_is_atan2_in_degrees() {
	vm().run_script(
		r#"
			local o = createVec(0, 0);
			// angle of the direction from o to each axis point, CCW from +X.
			assert(Math.getAngleTo(o, createVec(1, 0)) == 0.0);     // east
			assert(Math.getAngleTo(o, createVec(0, 1)) == 90.0);    // +Y
			assert(Math.getAngleTo(o, createVec(-1, 0)) == 180.0);  // west
			assert(Math.getAngleTo(o, createVec(0, -1)) == -90.0);  // -Y
			// not from the origin: direction (1,1) from (2,2) is 45 degrees.
			assert(Math.getAngleTo(createVec(2, 2), createVec(3, 3)) == 45.0);
			"#,
		"get_angle_to",
	)
	.expect("getAngleTo = atan2(dy, dx) in degrees");
}

#[test]
fn math_rand_is_in_range_and_seed_is_reproducible() {
	vm().run_script(
		r#"
			// zero-arg range
			for (local i = 0; i < 50; i = i + 1) {
				local r = Math.rand();
				assert(r >= 0 && r <= 32767);
			}
			// two-arg inclusive range
			for (local i = 0; i < 50; i = i + 1) {
				local r = Math.rand(1, 6);
				assert(r >= 1 && r <= 6);
			}
			// seeding makes the sequence reproducible
			Math.seedRandom(12345);
			local a = Math.rand(0, 1000000);
			local b = Math.rand(0, 1000000);
			Math.seedRandom(12345);
			assert(Math.rand(0, 1000000) == a);
			assert(Math.rand(0, 1000000) == b);
			// string seeding works and is deterministic
			Math.seedRandomString("hello");
			local c = Math.rand(0, 1000000);
			Math.seedRandomString("hello");
			assert(Math.rand(0, 1000000) == c);
			"#,
		"math_rand",
	)
	.expect("Math.rand range + reproducible seeding");
}

#[test]
fn math_rand_degenerate_and_extreme_ranges() {
	vm().run_script(
		r#"
			assert(Math.rand(5, 5) == 5);     // min == max
			assert(Math.rand(10, 3) == 10);   // max < min -> min
			// the full i64 range must not panic (i128-wide span) and stay in bounds
			local lo = -9223372036854775807 - 1;   // i64::MIN
			local hi = 9223372036854775807;        // i64::MAX
			local r = Math.rand(lo, hi);
			assert(r >= lo);
			"#,
		"rand_degenerate",
	)
	.expect("degenerate + extreme rand ranges are safe");
}

#[test]
fn string_table_and_helpers() {
	vm()
		.run_script(
			r#"
			assert(String.contains("hello world", "o w"));
			assert(!String.contains("hello", "z"));
			assert(String.replace("a.b.c", ".", "-") == "a-b-c");
			assert(String.remove("foo_bar_baz", "_bar") == "foo_baz");
			// empty needle is a no-op (the guards)
			assert(String.replace("abc", "", "X") == "abc");
			assert(String.remove("abc", "") == "abc");

			// %token% substitution from an array of [name, value] pairs
			local out = buildTextFromTemplate("Hi %name%, you are %role%", [["name", "Bob"], ["role", "sergeant"]]);
			assert(out == "Hi Bob, you are sergeant");

			// first-character membership
			assert(isFirstCharacter("Apple", ["A", "E", "I"]));
			assert(!isFirstCharacter("Banana", ["A", "E", "I"]));

			assert(removeFromBeginningOfText("The ", "The Keep") == "Keep");
			assert(removeFromBeginningOfText("The ", "A Keep") == "A Keep");

			assert(hexStringToInt("ff") == 255);
			assert(hexStringToInt("0x10") == 16);
			"#,
			"strings",
		)
		.expect("String table + helpers");
}

#[test]
fn build_text_from_template_stringifies_numbers() {
	// Regression: numeric vars (the common vanilla case — reward/kill counts) must
	// be substituted, not dropped as raw %tokens%.
	vm()
		.run_script(
			r#"
			assert(buildTextFromTemplate("have %n% heads", [["n", 3]]) == "have 3 heads");
			assert(buildTextFromTemplate("(%k%/%m%)", [["k", 2], ["m", 5]]) == "(2/5)");
			assert(buildTextFromTemplate("%who% won %gold% gold", [["who", "Bob"], ["gold", 100]]) == "Bob won 100 gold");
			// a missing key leaves the token untouched
			assert(buildTextFromTemplate("hi %name%", []) == "hi %name%");
			"#,
			"build_text_nums",
		)
		.expect("numeric template substitution");
}

#[test]
fn squirrel_stdlib_string_and_math_are_available() {
	vm().run_script(
		r#"
			// these come from the registered Squirrel stdlib, not our tables
			assert(format("%d-%s", 3, "x") == "3-x");
			local parts = split("a,b,c", ",");
			assert(parts.len() == 3 && parts[1] == "b");
			assert(strip("  hi  ") == "hi");
			assert(::sqrt(16.0) == 4.0);
			"#,
		"stdlib",
	)
	.expect("Squirrel stdlib string/math registered");
}

#[test]
fn platform_stubs_behave() {
	vm().run_script(
		r#"
			assert(isReleaseBuild() == true);
			assert(isSteamBuild() == false);
			assert(isDevmode() == false);
			assert(hasDLC(123) == false);   // default config owns no DLC
			assert(hasAchievement("x") == false);
			updateAchievement("x", 1, 10);   // no-op, must not error
			storeStatistics();
			__ping();
			crash("anything");               // accepts any arg, no-op
			crash();                         // zero args
			crash(1, true, "x", null);       // multiple mixed-type args
			reloadTextures();

			setPersistentStat("CrisesDefeatedOnIronman", 5);
			assert(getPersistentStat("CrisesDefeatedOnIronman") == 5);
			setPersistentStat("CrisesDefeatedOnIronman", 7);   // overwrite
			assert(getPersistentStat("CrisesDefeatedOnIronman") == 7);
			assert(getPersistentStat("Unknown") == 0);

			assert(typeof GameInfo.getVersionNumber() == "string");
			assert(GameInfo.getCompany() == "(c) Overhype Studios GmbH"); // as the original reports
			assert(GameInfo.getVersionNumber() == "1.5.2.3");
			"#,
		"platform",
	)
	.expect("platform stubs");
}

#[test]
fn world_get_time_exposes_constants_and_boot_defaults() {
	vm().run_script(
		r#"
			local t = World.getTime();
			// The original's day (probed 2026-10-13), read by config at load time.
			assert(t.SecondsPerDay == 105);
			assert(t.SecondsPerHour == 4.375);
			// A fresh world reads Day 1, 0:10, Dawn.
			assert(t.Time == 105.75);
			assert(t.Days == 1 && t.Hours == 0 && t.Minutes == 10 && t.TimeOfDay == 0);
			assert(t.IsDaytime == true);
			// One live object, like the original's.
			assert(World.getTime() == t);
			// usable in the float arithmetic config does (e.g. 1.0 / SecondsPerDay)
			assert(1.0 / World.getTime().SecondsPerDay > 0.0);
			"#,
		"world_time",
	)
	.expect("World.getTime constants + boot defaults");
}

#[test]
fn native_config_drives_build_flags() {
	let mut vm = SQVM::new(4096);
	let config = NativeConfig {
		is_steam_build: true,
		is_devmode: true,
		owned_dlc: vec![17, 42],
		version_number: "9.9.9".to_string(),
		..NativeConfig::default()
	};
	config.install(&mut vm);

	vm.run_script(
		r#"
		assert(isSteamBuild() == true);
		assert(isDevmode() == true);
		assert(hasDLC(17) == true);     // owned
		assert(hasDLC(42) == true);     // owned
		assert(hasDLC(99) == false);    // not owned
		assert(GameInfo.getVersionNumber() == "9.9.9");
		"#,
		"config",
	)
	.expect("NativeConfig drives build flags");
}

#[test]
fn logging_routes_to_the_host_handler() {
	let mut vm = SQVM::new(4096);
	install_natives(&mut vm);

	let captured: Rc<RefCell<Vec<(LogLevel, String)>>> = Rc::new(RefCell::new(Vec::new()));
	let sink = captured.clone();
	install_logging(&mut vm, move |level, msg| {
		sink.borrow_mut().push((level, msg.to_string()))
	});

	vm.run_script(
		r#"
		this.logInfo("info msg");
		this.logDebug("debug msg");
		this.logWarning("warn msg");
		this.logError("error msg");
		this.error("script error");
		this.logBeginGroup("g");   // no-op, must not error
		this.printStack();
		this.logEndGroup();
		"#,
		"logging",
	)
	.expect("logging routes without error");

	let log = captured.borrow();
	assert_eq!(log.len(), 5, "five level-bearing log calls were captured");
	assert_eq!(log[0], (LogLevel::Info, "info msg".to_string()));
	assert_eq!(log[1], (LogLevel::Debug, "debug msg".to_string()));
	assert_eq!(log[2], (LogLevel::Warning, "warn msg".to_string()));
	assert_eq!(log[3], (LogLevel::Error, "error msg".to_string()));
	assert_eq!(log[4], (LogLevel::Error, "script error".to_string()));
}

#[test]
fn install_logging_overrides_the_previous_handler() {
	let mut vm = SQVM::new(2048);
	let first: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
	let second: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));

	let f = first.clone();
	install_logging(&mut vm, move |_level, msg| {
		f.borrow_mut().push(msg.to_string())
	});
	let s = second.clone();
	install_logging(&mut vm, move |_level, msg| {
		s.borrow_mut().push(msg.to_string())
	});

	vm.run_script(r#"logInfo("hello");"#, "override")
		.expect("ok");
	assert!(first.borrow().is_empty(), "the first handler was replaced");
	assert_eq!(
		second.borrow().len(),
		1,
		"only the last handler receives logs"
	);
}
