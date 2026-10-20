//! Closing a VM whose own objects hold [`SqStrongRef`]s.
//!
//! An object the VM owns can carry Rust state that holds strong refs: a native
//! closure's captures, a native class's member slots. When such an object is only
//! kept alive by a reference cycle, `sq_close` frees it last, after it has already
//! freed the root VM and emptied the reference table. Dropping those refs must then
//! release nothing. Releasing them read the freed VM (a use-after-free).
//!
//! The old code passed these tests in a normal build. Its fault only showed under
//! AddressSanitizer, with the `SQ_ASAN` recipe on `squirrel-sys/build.rs`:
//! `SQ_ASAN=1 RUSTFLAGS="-Clink-arg=-L/usr/lib/gcc/x86_64-linux-gnu/15 -Clink-arg=-lasan"
//! cargo test -p squirrel --lib close_tests`, run with
//! `LD_PRELOAD=/usr/lib/gcc/x86_64-linux-gnu/15/libasan.so`.

use std::rc::Rc;

use crate::{ClassBuilder, SQVM, SqStrongRef};

/// A native closure that captures a strong ref, kept alive only by a table that
/// holds itself.
#[test]
fn closure_in_a_cycle_releases_its_capture_at_close() {
	let mut vm = SQVM::new(1024);
	let dropped = Rc::new(());

	let held = SqStrongRef::string(&vm, "kept by a VM-owned closure");
	let witness = dropped.clone();
	vm.register("holder", move || {
		let _ = (&held, &witness);
	});
	vm.run_script(
		r#"
		local cycle = { f = ::holder };
		cycle.self <- cycle;
		delete ::holder;
		"#,
		"close_tests",
	)
	.expect("the cycle is built");
	assert_eq!(Rc::strong_count(&dropped), 2, "the cycle keeps the closure alive");

	drop(vm);
	assert_eq!(Rc::strong_count(&dropped), 1, "closing the VM drops the closure's captures");
}

#[derive(Clone)]
struct Point {
	x: f64,
}

/// A native class's member slots hold strong refs to their accessor closures. An
/// instance in a cycle keeps the class alive until the end of `sq_close` (the case
/// the engine's own boot hit).
#[test]
fn class_instance_in_a_cycle_releases_its_slots_at_close() {
	let mut vm = SQVM::new(1024);
	let point = ClassBuilder::<Point>::new("Point")
		.property("X", |p| p.x, |p, x| p.x = x)
		.method("twice", |p: &mut Point| p.x * 2.0)
		.build(&mut vm);
	vm.register("makePoint", move |vm: &SQVM, x: f64| point.instantiate(vm, Point { x }));
	vm.run_script(
		r#"
		local cycle = { p = makePoint(2.0) };
		assert(cycle.p.X == 2.0 && cycle.p.twice() == 4.0);
		cycle.self <- cycle;
		delete ::makePoint;
		delete ::Point;
		"#,
		"close_tests",
	)
	.expect("the cycle is built");

	drop(vm);
}
