//! The Sqrat-style overload mechanism (`__sqrat_ol_ name_N` leaves + a strict
//! dispatcher), exercised through `Math.rand` — the canonical overloaded native
//! (`rand()` / `rand(min, max)`). Behaviour was pinned against the original engine.

use squirrel::SQVM;
use squirrel_bb::install_natives;

fn vm() -> SQVM {
	let mut vm = SQVM::new(2048);
	install_natives(&mut vm);
	vm
}

#[test]
fn rand_dispatches_by_arity() {
	vm().run_script(
		r#"
		this.Math.seedRandom(1);
		local a = this.Math.rand();       // 0-arg overload -> __sqrat_ol_ rand_0
		local b = this.Math.rand(5, 10);  // 2-arg overload -> __sqrat_ol_ rand_2
		assert(typeof a == "integer");
		assert(b >= 5 && b <= 10);
		"#,
		"rand_arity",
	)
	.expect("both rand overloads dispatch by argument count");
}

#[test]
fn overload_leaves_carry_the_exact_sqrat_keys() {
	// modern_hooks discovers overloads via regexp("^__sqrat_ol_ \\w+_(\\d+)$"); the
	// leaves must carry that exact key, including the single space after the prefix.
	vm().run_script(
		r#"
		assert("__sqrat_ol_ rand_0" in this.Math);
		assert("__sqrat_ol_ rand_2" in this.Math);
		assert(typeof this.Math["__sqrat_ol_ rand_2"] == "function");
		"#,
		"rand_leaf_keys",
	)
	.expect("leaves carry the exact __sqrat_ol_ keys");
}

#[test]
fn unregistered_arity_is_a_strict_error() {
	// rand has only _0 and _2; a 1-arg call has no leaf, so the dispatcher raises the
	// original engine's exact error rather than guessing a near match.
	vm().run_script(r#"this.Math.rand(5);"#, "rand_bad_arity")
		.expect_err("an unregistered arity must error, matching the original dispatcher");
}

#[test]
fn a_mod_can_wrap_a_single_overload() {
	// Replace only the 2-arg leaf. Because the dispatcher re-resolves the leaf on the
	// live receiver every call, rand(min, max) must route to the replacement while the
	// 0-arg overload stays untouched — the property real hook mods depend on.
	vm().run_script(
		r#"
		this.Math["__sqrat_ol_ rand_2"] = function(_min, _max) { return 999; };
		assert(this.Math.rand(1, 2) == 999);   // wrapped overload is honoured
		assert(typeof this.Math.rand() == "integer"); // other overload still works
		"#,
		"rand_wrap",
	)
	.expect("a mod replacing one overload leaf is honoured by the dispatcher");
}
