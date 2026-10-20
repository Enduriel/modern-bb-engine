use std::rc::Rc;

use super::{ClassBuilder, Native};
use crate::SQVM;

#[derive(Clone)]
struct Point {
	x: f64,
	y: f64,
}

#[test]
fn native_class_round_trips_through_script() {
	let mut vm = SQVM::new(1024);

	let point = ClassBuilder::<Point>::new("Point")
		.property("X", |p| p.x, |p, x| p.x = x)
		.property("Y", |p| p.y, |p, y| p.y = y)
		.build(&mut vm);

	// Factory native function creating instances with a Rust payload.
	vm.register("makePoint", move |vm: &SQVM, x: f64, y: f64| {
		point.instantiate(vm, Point { x, y })
	});
	// A native function reading a Point instance back into its payload.
	vm.register("sumXY", |p: Native<Point>| p.0.x + p.0.y);

	vm.run_script(
		r#"
		local p = makePoint(3.0, 4.0);
		assert(typeof p == "instance");
		assert(p instanceof Point);

		// _get reads the payload:
		assert(p.X == 3.0);
		assert(p.Y == 4.0);

		// _set writes the payload, _get reflects it:
		p.X = 10.0;
		assert(p.X == 10.0);

		// the payload read back natively sees the mutation: 10 + 4
		assert(sumXY(p) == 14.0);

		// distinct instances have independent payloads:
		local q = makePoint(1.0, 1.0);
		p.X = 99.0;
		assert(q.X == 1.0);
		"#,
		"native_class_test",
	)
	.expect("native class round-trips through script");
}

#[test]
fn rejects_wrong_argument_types() {
	let mut vm = SQVM::new(1024);
	let point = ClassBuilder::<Point>::new("Point")
		.property("X", |p| p.x, |p, x| p.x = x)
		.property("Y", |p| p.y, |p, y| p.y = y)
		.build(&mut vm);
	vm.register("makePoint", move |vm: &SQVM, x: f64, y: f64| {
		point.instantiate(vm, Point { x, y })
	});
	vm.register("sumXY", |p: Native<Point>| p.0.x + p.0.y);

	// The 'x' type mask rejects a non-instance argument before the closure runs.
	vm.run_script(r#"sumXY(5);"#, "wrong_type")
		.expect_err("an integer is not an instance");

	// Reading an unknown property errors via the _get metamethod.
	vm.run_script(r#"local p = makePoint(1.0, 2.0); return p.Z;"#, "bad_prop")
		.expect_err("Z is not a property");
}

#[test]
fn numeric_property_setter_reads_a_non_number_as_zero() {
	// Sqrat's property setters read the value through the same `Var<T>` as native
	// arguments: a non-number writes 0 instead of raising.
	let mut vm = SQVM::new(1024);
	let point = ClassBuilder::<Point>::new("Point")
		.property("X", |p| p.x, |p, x| p.x = x)
		.build(&mut vm);
	vm.register("makePoint", move |vm: &SQVM, x: f64| point.instantiate(vm, Point { x, y: 0.0 }));

	vm.run_script(
		r#"
		local p = makePoint(3.0);
		p.X = 2;
		assert(p.X == 2.0);
		p.X = null;
		assert(p.X == 0.0);
		p.X = 5.0;
		p.X = "7";
		assert(p.X == 0.0);
		"#,
		"lenient_setter",
	)
	.expect("a non-number property write reads as zero");
}

#[test]
fn native_arg_rejects_a_different_native_class() {
	// Two native classes share one typetag but have distinct payload TypeIds.
	// Passing a B where an A is expected must be rejected by the TypeId check —
	// this is the line that prevents a type-confused read of one payload as
	// another, and it would otherwise have no coverage.
	#[derive(Clone)]
	struct A {
		v: i64,
	}
	#[derive(Clone)]
	struct B {
		v: i64,
	}

	let mut vm = SQVM::new(1024);
	let a = ClassBuilder::<A>::new("A")
		.property("v", |x| x.v, |x, v| x.v = v)
		.build(&mut vm);
	let b = ClassBuilder::<B>::new("B")
		.property("v", |x| x.v, |x, v| x.v = v)
		.build(&mut vm);
	vm.register("makeA", move |vm: &SQVM| a.instantiate(vm, A { v: 1 }));
	vm.register("makeB", move |vm: &SQVM| b.instantiate(vm, B { v: 2 }));
	vm.register("takeA", |x: Native<A>| x.0.v);

	// A `B` passes the `x` (instance) mask but fails the TypeId check inside
	// `Native::<A>::from_sq`, so the call errors instead of misreading memory.
	vm.run_script("takeA(makeB());", "wrong_class")
		.expect_err("a B is not an A");
	// A correct A still works afterward (the VM stayed usable).
	vm.run_script("assert(takeA(makeA()) == 1);", "right_class")
		.expect("an A is accepted");
}

#[test]
fn live_instance_payload_dropped_on_vm_close() {
	#[derive(Clone)]
	struct Tracked(#[allow(dead_code)] Rc<()>);

	let marker = Rc::new(());
	let captured = marker.clone(); // marker + this = 2
	{
		let mut vm = SQVM::new(1024);
		let cls = ClassBuilder::<Tracked>::new("Tracked").build(&mut vm);
		vm.register("makeTracked", move |vm: &SQVM| {
			cls.instantiate(vm, Tracked(captured.clone()))
		});
		// Stash an instance in the root table so it outlives the script and stays
		// alive until the VM closes.
		vm.run_script("::held <- makeTracked();", "hold")
			.expect("held");
		assert_eq!(
			Rc::strong_count(&marker),
			3,
			"alive: marker + closure capture + held instance's payload"
		);
		// Dropping the VM closes it, which must fire the release hook on the still
		// -live instance (and free the closure that captured the other clone).
	}
	assert_eq!(
		Rc::strong_count(&marker),
		1,
		"closing the VM dropped both the live instance's payload and the closure"
	);
}

#[test]
fn many_instances_all_release_their_payloads() {
	use std::cell::Cell;

	#[derive(Clone)]
	struct Counted(Rc<Cell<usize>>);
	impl Drop for Counted {
		fn drop(&mut self) {
			self.0.set(self.0.get() + 1);
		}
	}

	let mut vm = SQVM::new(2048);
	let counter = Rc::new(Cell::new(0usize));
	let cls = ClassBuilder::<Counted>::new("Counted").build(&mut vm);
	let captured = counter.clone();
	vm.register("makeCounted", move |vm: &SQVM| {
		cls.instantiate(vm, Counted(captured.clone()))
	});

	// Create and immediately discard 1000 instances; each payload must be dropped
	// exactly once when its instance is collected.
	vm.run_script(
		"for (local i = 0; i < 1000; i = i + 1) { local c = makeCounted(); }",
		"stress",
	)
	.expect("stress run");
	assert_eq!(
		counter.get(),
		1000,
		"every instance payload was dropped once"
	);
}

#[test]
fn instance_payload_is_dropped_when_instance_dies() {
	#[derive(Clone)]
	struct Tracked(#[allow(dead_code)] Rc<()>);

	let mut vm = SQVM::new(1024);
	let tracked = ClassBuilder::<Tracked>::new("Tracked").build(&mut vm);

	let marker = Rc::new(());
	let captured = marker.clone();
	vm.register("makeTracked", move |vm: &SQVM| {
		tracked.instantiate(vm, Tracked(captured.clone()))
	});

	// marker + the closure-captured clone.
	assert_eq!(Rc::strong_count(&marker), 2);

	// Create an instance inside a block; it dies at the block's end, which frees
	// the instance and runs the release hook that drops its payload.
	vm.run_script(r#"{ local t = makeTracked(); }"#, "make")
		.expect("instance created and released");

	assert_eq!(
		Rc::strong_count(&marker),
		2,
		"the payload clone was dropped when the instance was collected"
	);
}

#[test]
fn native_methods_mutate_the_payload_and_coexist_with_properties() {
	let mut vm = SQVM::new(1024);

	let point = ClassBuilder::<Point>::new("Point")
		.property("X", |p| p.x, |p, x| p.x = x)
		.property("Y", |p| p.y, |p, y| p.y = y)
		// A mutating method (typed args, returns nothing) ...
		.method("translate", |p: &mut Point, dx: f64, dy: f64| {
			p.x += dx;
			p.y += dy;
		})
		// ... and a method that returns a value.
		.method("sum", |p: &mut Point| p.x + p.y)
		.build(&mut vm);
	vm.register("makePoint", move |vm: &SQVM, x: f64, y: f64| {
		point.instantiate(vm, Point { x, y })
	});

	vm.run_script(
		r#"
		local p = makePoint(1.0, 2.0);
		// the method resolves ahead of the _get fallback ...
		p.translate(10.0, 20.0);
		// ... and mutated the very same payload the properties read:
		assert(p.X == 11.0);
		assert(p.Y == 22.0);
		// a value-returning method works too:
		assert(p.sum() == 33.0);
		// properties still route through _get, which still rejects unknowns:
		assert(p.X == 11.0);
		"#,
		"native_method_test",
	)
	.expect("native methods mutate the payload and coexist with properties");

	// An unknown member still errors via _get (methods didn't shadow the fallback).
	vm.run_script(r#"makePoint(0.0, 0.0).nope;"#, "unknown_member")
		.expect_err("an unknown member still errors");
}

#[test]
fn host_reads_and_writes_an_instance_payload() {
	let mut vm = SQVM::new(1024);
	let point = ClassBuilder::<Point>::new("Point")
		.property("X", |p| p.x, |p, x| p.x = x)
		.property("Y", |p| p.y, |p, y| p.y = y)
		.build(&mut vm);
	let other = ClassBuilder::<u8>::new("Other").build(&mut vm);
	let instance = point.instantiate(&vm, Point { x: 1.0, y: 2.0 });

	// Script writes, host reads; host writes, script reads.
	crate::SqTable::root(&vm).set("p", instance.clone());
	vm.run_script("::p.X = 5.0;", "set").unwrap();
	assert_eq!(point.with_payload(&vm, &instance, |p| p.x), Some(5.0));
	point.with_payload(&vm, &instance, |p| p.y = 9.0);
	vm.run_script("assert(::p.Y == 9.0);", "check").unwrap();
	// The ref keeps where the payload is: later script writes still show, through it
	// and its clones.
	vm.run_script("::p.X = 7.0;", "set again").unwrap();
	assert_eq!(point.with_payload(&vm, &instance, |p| p.x), Some(7.0));
	assert_eq!(point.with_payload(&vm, &instance.clone(), |p| p.x), Some(7.0));

	// A different payload type (or a non-instance) yields None.
	assert_eq!(other.with_payload(&vm, &instance, |v| *v), None);
	let table = crate::SqTable::new(&vm).to_strong();
	assert!(point.with_payload(&vm, &table, |p| p.x).is_none());
}

/// Sqrat's layout, which mods rely on: the class's `__getTable`/`__setTable` hold the
/// property accessors, and replacing one (as Reforged wraps `TacticalTile.IsEmpty`)
/// changes every access; a class method can be replaced with `<-` before instances
/// exist.
#[test]
fn mods_can_wrap_properties_and_methods_like_sqrat() {
	let mut vm = SQVM::new(1024);
	let point = ClassBuilder::<Point>::new("Point")
		.property("X", |p| p.x, |p, x| p.x = x)
		.property_obj("Pair", |p, vm| {
			let mut t = crate::SqTable::new(vm);
			t.set("x", p.x);
			t.to_strong()
		})
		.method("sum", |p: &mut Point| p.x + p.y)
		.build(&mut vm);
	vm.register("makePoint", move |vm: &SQVM, x: f64, y: f64| point.instantiate(vm, Point { x, y }));
	vm.run_script(
		r#"
		assert(typeof ::Point.__getTable.X == "function" || typeof ::Point.__getTable.X == "native function");
		assert("X" in ::Point.__setTable);
		assert(!("Pair" in ::Point.__setTable)); // read-only: no setter

		// A method replaced on the class before any instance exists.
		local sum = ::Point.sum;
		::Point.sum <- function() { return sum() * 10; };

		// Wrap a getter and a setter the way mods do.
		local getX = ::Point.__getTable.X;
		::Point.__getTable.X <- function() { return getX() + 100; };
		local setX = ::Point.__setTable.X;
		::Point.__setTable.X <- function( _v ) { setX(_v * 2); };
		// Add a property.
		::Point.__getTable.Double <- function() { return this.X * 2; };

		local p = makePoint(3.0, 4.0);
		assert(p.X == 103.0);
		p.X = 5.0;               // stored as 10
		assert(p.X == 110.0);
		assert(p.Double == 220.0);
		assert(p.sum() == 140.0); // (10 + 4) * 10
		assert(p.Pair.x == 10.0);
		local threw = false;
		try { p.Pair = 1; } catch (e) { threw = true; }
		assert(threw);
		threw = false;
		try { local v = p.Missing; } catch (e) { threw = true; }
		assert(threw);
		"#,
		"sqrat_layout",
	)
	.expect("properties and methods are hookable like Sqrat's");
}

/// A missing property is a clean miss, so the instance's default delegate still answers
/// (`weakref`, `getclass`), and anything else is Squirrel's own "does not exist".
#[test]
fn missing_properties_fall_back_to_the_default_delegate() {
	let mut vm = SQVM::new(1024);
	let point = ClassBuilder::<Point>::new("Point").property("X", |p| p.x, |p, x| p.x = x).build(&mut vm);
	vm.register("makePoint", move |vm: &SQVM, x: f64, y: f64| point.instantiate(vm, Point { x, y }));
	vm.run_script(
		r#"
		local p = makePoint(1.0, 2.0);
		local w = p.weakref();
		assert(w.ref() == p);
		assert(p.getclass() == ::Point);
		local msg = null;
		try { local v = p.Missing; } catch (e) { msg = e; }
		assert(msg == "the index 'Missing' does not exist");
		"#,
		"default_delegate",
	)
	.expect("missing keys reach the default delegate");
}
