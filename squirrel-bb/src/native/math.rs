//! The BB `Math` table.
//!
//! Distinct from Squirrel's stdlib math (which the VM already registers at the
//! root as `::sin`/`::sqrt`/… returning floats). BB's `Math.ceil`/`floor` return
//! **floats** (callers tointeger() them at array-index sites) but `Math.round` returns
//! an **integer**; `Math.min`/`max` and `Math.abs` are integer-only (with `minf`/`maxf`/
//! `absf` for floats). Checked against the original (parity probes `menu/math`). The
//! RNG (`rand`/`seedRandom`/`seedRandomString`) shares one [`Prng`] state, created
//! once by the native installer and shared with `color.varyRGB`.

use squirrel::{DynNumber, Native, Number, SQVM, SqStrongRef, SqTable};

use super::overload::{make_dispatcher, overload_key};
use super::prng::Prng;
use super::vec::Vec2f;

/// `Math.round`: in f32, add (or below zero subtract) 0.5 and truncate toward zero.
fn round_f32(x: f64) -> i64 {
	let x = x as f32;
	let shifted = if x >= 0.0 { x + 0.5 } else { x - 0.5 };
	cvtt_i32(shifted)
}

/// `Math.getAngleTo`: see the registration.
fn angle_to(a: &Vec2f, b: &Vec2f) -> f64 {
	let (dx, dy) = ((b.x as f32) - (a.x as f32), (b.y as f32) - (a.y as f32));
	// The original's atan2 runs in double and rounds the result to f32.
	let mut angle = ((dx as f64).atan2(dy as f64) as f32) * 57.29578_f32;
	if angle < 0.0 {
		angle += 360.0;
	}
	f64::from(-angle)
}

/// x86 `cvttss2si`: truncate toward zero; NaN or out of `i32` range is `i32::MIN`.
fn cvtt_i32(x: f32) -> i64 {
	if x.is_nan() || x >= 2_147_483_648.0 || x < -2_147_483_648.0 {
		i64::from(i32::MIN)
	} else {
		i64::from(x as i32)
	}
}

/// Register the `Math` table into the root, using the shared [`Prng`] for `rand`.
pub(crate) fn install(vm: &mut SQVM, prng: Prng) {
	// Build every closure first: `make_closure` borrows the VM mutably, so it must
	// finish before the `SqTable` handles (which hold a shared borrow) are made.

	// `Math.rand` is a Sqrat overload: `rand()` (0 args) and `rand(min, max)` (2 args).
	// Register the two leaves under their exact `__sqrat_ol_ rand_N` keys plus a strict
	// dispatcher under `rand`, so a mod can wrap one arity and the dispatcher honours it.
	let rand_0 = {
		let p = prng.clone();
		vm.make_closure(move || -> i64 { p.rand_default() })
	};
	let rand_2 = {
		let p = prng.clone();
		vm.make_closure(move |min: i64, max: i64| -> i64 { p.rand_range(min, max) })
	};
	let rand_dispatch = make_dispatcher(vm, "rand");

	let entries: Vec<(&str, SqStrongRef)> = vec![
		("seedRandom", {
			let p = prng.clone();
			// Accepts int or float (e.g. seeded from Time.getRealTime()).
			vm.make_closure(move |seed: DynNumber| p.seed(seed.as_i64()))
		}),
		("seedRandomString", {
			let p = prng.clone();
			vm.make_closure(move |text: &str| p.seed_str(text))
		}),
		// Integer variants: arguments truncate to i64, compare, return i64.
		(
			"min",
			vm.make_closure(|a: DynNumber, b: DynNumber| a.as_i64().min(b.as_i64())),
		),
		(
			"max",
			vm.make_closure(|a: DynNumber, b: DynNumber| a.as_i64().max(b.as_i64())),
		),
		// Float variants.
		(
			"minf",
			vm.make_closure(|a: DynNumber, b: DynNumber| a.as_f64().min(b.as_f64())),
		),
		(
			"maxf",
			vm.make_closure(|a: DynNumber, b: DynNumber| a.as_f64().max(b.as_f64())),
		),
		// abs is an integer function like min/max: a float truncates first, so
		// abs(0.798) == 0 (probed 2026-10-13; the AI's armor-priority term relies on it).
		// `absf` is the float one. The original binds the C runtime's `abs(int)`.
		("abs", vm.make_closure(|x: DynNumber| x.as_i64().wrapping_abs())),
		("absf", vm.make_closure(|x: Number| x.0.abs())),
		// ceil/floor return FLOATS in BB (verified: actor.updateOverlay does an
		// unguarded `getHitpoints() / getHitpointsMax()` where getHitpointsMax() is
		// `Math.floor(...)`; that survives a 0 denominator only as float division
		// (1/0.0 = inf, then clamped by minf). Squirrel array indexing tointeger()s a
		// float key (sqvm.cpp OT_ARRAY branch), so `arr[Math.floor(x)]` still works.
		("ceil", vm.make_closure(|x: Number| x.0.ceil())),
		("floor", vm.make_closure(|x: Number| x.0.floor())),
		// round returns an INTEGER: `trunc(x + 0.5)` (or `- 0.5` below zero) in f32, as
		// the original computes it (disassembled at `0x5370a0`), f32 rounding included.
		("round", vm.make_closure(|x: Number| round_f32(x.0))),
		(
			"pow",
			vm.make_closure(|base: Number, exp: Number| base.0.powf(exp.0)),
		),
		// getAngleTo(v1, v2): the compass bearing from v1 to v2 (atan2(dx, dy), so north
		// is 0 and east 90), normalised to [0, 360) and then NEGATED, in f32, as the
		// original computes it (disassembled at `0x537100`): east is -90, north -0.
		(
			"getAngleTo",
			vm.make_closure(|a: Native<Vec2f>, b: Native<Vec2f>| angle_to(&a.0, &b.0)),
		),
	];

	let mut math = SqTable::new(vm);
	for (name, func) in entries {
		math.set(name, func);
	}
	math.set(&overload_key("rand", 0), rand_0);
	math.set(&overload_key("rand", 2), rand_2);
	math.set("rand", rand_dispatch);
	SqTable::root(vm).set("Math", math);
}
