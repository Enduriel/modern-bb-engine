//! End-to-end tests for the native value classes (`vec2f`/`vec2i`/`color`) and
//! their factory functions, driven the way scripts use them.

use squirrel::SQVM;
use squirrel_bb::install_natives;

fn vm() -> SQVM {
	let mut vm = SQVM::new(2048);
	install_natives(&mut vm);
	vm
}

#[test]
fn vec2f_creates_reads_mutates_and_coerces_ints() {
	vm().run_script(
		r#"
			local v = createVec(3, 4);          // integer args coerced to float
			assert(typeof v == "instance");
			assert(v instanceof vec2f);
			assert(v.X == 3.0);
			assert(v.Y == 4.0);

			v.X = 10.5;                          // in-place mutation via _set
			assert(v.X == 10.5);                 // _get reflects it

			local w = createVec(0.0, 0.0);
			assert(v instanceof vec2f && w instanceof vec2f);
			"#,
		"vec_basic",
	)
	.expect("vec2f basics work");
}

#[test]
fn vec_distance_and_direction() {
	vm().run_script(
		r#"
			// classic 3-4-5 triangle
			assert(getVecDistance(createVec(0, 0), createVec(3, 4)) == 5.0);

			// getVecDir returns a normalized direction
			local d = getVecDir(createVec(0, 0), createVec(0, 10));
			assert(d instanceof vec2f);
			assert(d.X == 0.0);
			assert(d.Y == 1.0);
			"#,
		"vec_math",
	)
	.expect("vec distance/direction work");
}

#[test]
fn vec_rejects_non_numbers() {
	vm().run_script(r#"createVec("a", 1);"#, "vec_bad")
		.expect_err("a string is not a number");
}

#[test]
fn color_parses_hex_and_keeps_channels_consistent() {
	vm().run_script(
		r##"
			local c = createColor("#ff8000");
			assert(c instanceof color);
			assert(c.R == 255);
			assert(c.G == 128);
			assert(c.B == 0);
			assert(c.A == 255);                 // '#rgb' form is opaque
			assert(c.RGBA == 0xffff8000);       // ARGB packed

			// 8-char rrggbbaa form carries explicit alpha
			local d = createColor("0080ff40");
			assert(d.R == 0 && d.G == 128 && d.B == 255 && d.A == 64);

			// setting a channel updates the packed value...
			c.R = 0;
			assert(c.RGBA == 0xff008000);
			// ...and setting the packed value updates the channels
			c.RGBA = 0x12345678;
			assert(c.A == 0x12 && c.R == 0x34 && c.G == 0x56 && c.B == 0x78);
			"##,
		"color_hex",
	)
	.expect("color parsing + channel consistency");
}

#[test]
fn native_arg_rejects_wrong_class_and_vm_stays_usable() {
	let vm = vm();
	// A color passes the `x` (instance) mask but is not a vec2f, so the TypeId
	// check rejects it instead of misreading its u32 as two f64s.
	vm.run_script(
		r##"local c = createColor("#ffffff"); getVecDistance(c, c);"##,
		"cross_type",
	)
	.expect_err("a color is not a vec2f");
	// The VM stays usable after the rejection.
	vm.run_script(
		"assert(getVecDistance(createVec(0, 0), createVec(3, 4)) == 5.0);",
		"after",
	)
	.expect("vm still usable");
}

#[test]
fn vec_dir_of_coincident_points_is_zero() {
	vm().run_script(
		r#"
			local z = getVecDir(createVec(5, 5), createVec(5, 5));
			assert(z.X == 0.0 && z.Y == 0.0);   // also rules out NaN
			"#,
		"vec_dir_zero",
	)
	.expect("coincident points give the zero vector");
}

#[test]
fn color_set_errors_are_recoverable_and_assignment_yields_the_value() {
	let vm = vm();
	// A non-number assigned to a numeric channel errors via _set.
	vm.run_script(
		r##"local c = createColor("#ffffff"); c.R = "x";"##,
		"set_wrong_type",
	)
	.expect_err("a string is not a valid channel value");
	// Assigning an unknown property errors via _set.
	vm.run_script(
		r##"local c = createColor("#ffffff"); c.Z = 5;"##,
		"set_unknown",
	)
	.expect_err("Z is not a property");
	// After those thrown errors the VM is still usable, and assignment yields the
	// assigned value (the SetMeta result-value contract).
	vm.run_script(
		r##"local c = createColor("#000000"); local x = (c.R = 7); assert(x == 7); assert(c.R == 7);"##,
		"set_chain",
	)
	.expect("assignment result + mutation, vm usable");
}

#[test]
fn color_channels_mask_out_of_range_values() {
	vm().run_script(
		r#"
			// Channels are 0-1 floats scaled by 255 into a wrapping byte (probed).
			local c = createColorRGB(300, 0, 0);   // 76500 & 0xff == 212
			assert(c.R == 212);
			c.A = -1;                               // (-1) & 0xff == 255
			assert(c.A == 255);
			"#,
		"channel_range",
	)
	.expect("channels mask to 0..255");
}

#[test]
fn color_rgb_and_white_factories() {
	vm().run_script(
		r#"
			local w = createColorWhite();
			assert(w.R == 255 && w.G == 255 && w.B == 255 && w.A == 255);

			// Channels are 0-1 floats (values probed in the original).
			local e = createColorRGB(0.5, 1.0, 0.25);
			assert(e.R == 127 && e.G == 255 && e.B == 63 && e.A == 255);
			"#,
		"color_factories",
	)
	.expect("color RGB/white factories");
}

#[test]
fn color_vary_rgb_is_identity_at_zero_and_seed_reproducible() {
	vm().run_script(
		r##"
			// factor 0 => each channel * 1.0 => unchanged (deterministic).
			local c = createColor("6496c8ff");
			c.varyRGB(0.0, 0.0, 0.0);
			assert(c.R == 100 && c.G == 150 && c.B == 200 && c.A == 255);

			// multiplicative jitter: zero channels stay zero however large the factor.
			local red = createColor("#ff0000");
			red.varyRGB(0.75, 0.75, 0.75);
			assert(red.G == 0 && red.B == 0);     // 0 * anything == 0
			assert(red.R >= 0 && red.R <= 255);    // clamped into range
			assert(red.A == 255);                  // alpha untouched

			// shares the seeded Math PRNG => reproducible.
			Math.seedRandom(42);
			local a = createColor("#ff0000"); a.varyRGB(0.5, 0.5, 0.5);
			Math.seedRandom(42);
			local b = createColor("#ff0000"); b.varyRGB(0.5, 0.5, 0.5);
			assert(a.R == b.R);
			"##,
		"vary_rgb",
	)
	.expect("varyRGB identity-at-zero, channel rules, and seeded reproducibility");
}

#[test]
fn to_hash_is_stable_per_object_and_distinct_across_objects() {
	vm().run_script(
		r#"
			local t = {};
			assert(typeof toHash(t) == "integer");
			assert(toHash(t) == toHash(t));            // stable for one object
			local u = {};
			assert(toHash(t) != toHash(u));            // distinct live objects differ

			// works on native instances too (item.getInstanceID's pattern):
			local v = createVec(1, 2);
			assert(toHash(v) == toHash(v));
			assert(typeof toHash(v).tostring() == "string");
			"#,
		"to_hash",
	)
	.expect("toHash is a stable, per-object integer");
}

#[test]
fn tile_transition_and_tactical_and_brush_natives() {
	vm().run_script(
		r#"
			// createTileTransition returns a configurable descriptor instance ...
			local t = createTileTransition();
			assert(typeof t == "instance");
			t.setSocket("socket_earth");
			t.setBlendIntoSockets(false);
			t.setBrush(0, "transition_road_N");   // direction ints (N..NW)
			t.setBrush(3, "transition_road_S");
			// ... handed to Tactical.setTransitions without error.
			Tactical.setTransitions("tile_road", t);

			// doesBrushExist reports false until a real atlas exists (callers only
			// gate optional brushes on it).
			assert(doesBrushExist("anything") == false);
			"#,
		"tile_transition",
	)
	.expect("createTileTransition + Tactical.setTransitions + doesBrushExist");
}

#[test]
fn color_overlong_hex_typo_is_transparent_black() {
	// vanilla has a few 9-char typos like "fffffff00": the original's hex read
	// overflows and leaves 0, so they are transparent black (probed).
	vm().run_script(
		r##"
			local c = createColor("fffffff00");
			assert(c.R == 0 && c.G == 0 && c.B == 0 && c.A == 0);
			local h = createColor("#ff804020");   // with '#': low 24 bits, alpha 255
			assert(h.R == 0x80 && h.G == 0x40 && h.B == 0x20 && h.A == 255);
			"##,
		"color_typo",
	)
	.expect("over-long hex reads as the original does");
}
