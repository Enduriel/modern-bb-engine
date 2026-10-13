//! `Math.rand` / `seedRandom` / `seedRandomString` against sequences captured from the
//! ORIGINAL game (2026-10-13). Seed compatibility depends on these
//! matching exactly: `seedRandom(X)` + worldgen must reproduce the vanilla map.

use squirrel::SQVM;

fn vm() -> SQVM {
	let mut vm = SQVM::new(1024);
	squirrel_bb::install_natives(&mut vm);
	vm
}

/// Run `body` (which must leave a comma-joined sequence in `out`) and compare.
fn assert_seq(setup: &str, n: usize, call: &str, expected: &[i64]) {
	let script = format!(
		r#"
		{setup}
		local s = "";
		for (local i = 0; i < {n}; i++) s += {call} + ",";
		::__out <- s;
		"#
	);
	let vm = vm();
	vm.run_script(&script, "prng_case").expect("script runs");
	let got = squirrel::SqTable::root(&vm)
		.get::<String>("__out")
		.expect("__out set");
	let want: String = expected.iter().map(|v| format!("{v},")).collect();
	assert_eq!(got, want, "setup `{setup}`, call `{call}`");
}

const R16: &str = "Math.rand(0, 65535)";

#[test]
fn integer_seeds_drive_mt19937_64_modulo() {
	assert_seq(
		"Math.seedRandom(0);",
		16,
		R16,
		&[56382, 63883, 59577, 64750, 9644, 59886, 57647, 18152, 17254, 18969, 13587, 63224, 34446, 62214, 25656, 12865],
	);
	assert_seq(
		"Math.seedRandom(1);",
		16,
		R16,
		&[28520, 64078, 17818, 49294, 26424, 26697, 53684, 19209, 24832, 32784, 61184, 13083, 7781, 23139, 988, 20889],
	);
	assert_seq(
		"Math.seedRandom(42);",
		8,
		"Math.rand(1, 100)",
		&[7, 25, 51, 63, 82, 29, 37, 45],
	);
	assert_seq(
		"Math.seedRandom(2147483647);",
		8,
		"Math.rand(0, 32767)",
		&[13579, 21785, 31118, 4870, 8862, 32469, 12151, 2314],
	);
}

#[test]
fn negative_and_float_seeds_truncate_to_u32() {
	// -1 -> 0xFFFFFFFF
	assert_seq(
		"Math.seedRandom(-1);",
		8,
		"Math.rand(0, 32767)",
		&[15182, 24919, 25515, 30078, 9905, 20163, 13488, 1442],
	);
	assert_seq(
		"Math.seedRandom(-2);",
		6,
		R16,
		&[53423, 62323, 58567, 42054, 33657, 23886],
	);
	// floats truncate toward zero: 1.7 -> 1, 2.9 -> 2, -1.5 -> -1
	assert_seq("Math.seedRandom(1.7);", 4, R16, &[28520, 64078, 17818, 49294]);
	assert_seq("Math.seedRandom(2.9);", 4, R16, &[35404, 12121, 12837, 11827]);
	assert_seq("Math.seedRandom(-1.5);", 4, R16, &[47950, 57687, 58283, 62846]);
}

#[test]
fn string_seeds_use_x65599_hash() {
	assert_seq("Math.seedRandomString(\"\");", 4, R16, &[56382, 63883, 59577, 64750]);
	assert_seq("Math.seedRandomString(\"a\");", 4, R16, &[40654, 25209, 15119, 60188]);
	assert_seq(
		"Math.seedRandomString(\"rust-engine-test\");",
		8,
		R16,
		&[5779, 59897, 46898, 15489, 61361, 59373, 63114, 62467],
	);
	assert_seq(
		"Math.seedRandomString(\"Battle Brothers\");",
		8,
		R16,
		&[14635, 46766, 48683, 58799, 34391, 34683, 61637, 47847],
	);
}

#[test]
fn ranges_negative_large_and_empty() {
	assert_seq(
		"Math.seedRandom(0);",
		8,
		"Math.rand(-5, 5)",
		&[3, 0, -3, 1, -5, -1, -4, -1],
	);
	assert_seq(
		"Math.seedRandom(0);",
		3,
		"Math.rand(0, 1000000000)",
		&[824498419, 696516320, 276316141],
	);
	assert_seq(
		"Math.seedRandom(0);",
		3,
		"Math.rand(-2147483648, 2147483647)",
		&[1262607422, -1103038069, 1882384569],
	);
	// float args truncate
	assert_seq(
		"Math.seedRandom(0);",
		8,
		"Math.rand(1.5, 3.7)",
		&[1, 3, 2, 1, 2, 3, 2, 1],
	);
	// an empty range returns `min` and consumes NO draw
	assert_seq(
		"Math.seedRandom(0); assert(Math.rand(5, 5) == 5); assert(Math.rand(10, 0) == 10);",
		4,
		R16,
		&[56382, 63883, 59577, 64750],
	);
}

#[test]
fn zero_arg_rand_is_a_separate_15_bit_stream() {
	// The original's 0-arg rand is the C runtime rand() (MSVC LCG), a separate stream:
	// drawing from it must not advance the seeded rand(a, b) stream.
	assert_seq(
		"Math.seedRandom(0); for (local i = 0; i < 5; i++) { local r = Math.rand(); assert(typeof r == \"integer\" && r >= 0 && r <= 32767); }",
		4,
		R16,
		&[56382, 63883, 59577, 64750],
	);
}
