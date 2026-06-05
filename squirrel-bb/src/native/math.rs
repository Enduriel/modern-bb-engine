//! The BB `Math` table.
//!
//! Distinct from Squirrel's stdlib math (which the VM already registers at the
//! root as `::sin`/`::sqrt`/… returning floats). BB's `Math.ceil`/`floor`/`round`
//! return **integers**, `Math.min`/`max` are the integer variants (with `minf`/
//! `maxf` for floats), and `Math.abs` is polymorphic (int→int, float→float). The
//! RNG (`rand`/`seedRandom`/`seedRandomString`) shares one [`Prng`] state, created
//! once by the native installer and shared with `color.varyRGB`.

use squirrel::{DynNumber, Native, Number, SQVM, SqStrongRef, SqTable};

use super::overload::{make_dispatcher, overload_key};
use super::prng::Prng;
use super::vec::Vec2f;

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
		// Polymorphic abs (int in → int out, float in → float out).
		(
			"abs",
			vm.make_closure(|x: DynNumber| match x {
				DynNumber::Int(n) => DynNumber::Int(n.wrapping_abs()),
				DynNumber::Float(f) => DynNumber::Float(f.abs()),
			}),
		),
		("absf", vm.make_closure(|x: Number| x.0.abs())),
		// ceil/floor/round return integers (the BB distinction from ::ceil/etc).
		("ceil", vm.make_closure(|x: Number| x.0.ceil() as i64)),
		("floor", vm.make_closure(|x: Number| x.0.floor() as i64)),
		("round", vm.make_closure(|x: Number| x.0.round() as i64)),
		(
			"pow",
			vm.make_closure(|base: Number, exp: Number| base.0.powf(exp.0)),
		),
		// getAngleTo(v1, v2): the angle in degrees of the direction from v1 to v2,
		// measured CCW from +X (standard atan2). The one caller adds its own +90 to
		// get a perpendicular label orientation, so that offset is not ours.
		(
			"getAngleTo",
			vm.make_closure(|a: Native<Vec2f>, b: Native<Vec2f>| {
				(b.0.y - a.0.y).atan2(b.0.x - a.0.x).to_degrees()
			}),
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
