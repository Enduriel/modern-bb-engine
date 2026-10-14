//! Time native property reads (`inst.X` through the class's `_get` and `__getTable`).
//!
//! Run: `cargo run --release -p squirrel --example prop_bench`

use squirrel::{ClassBuilder, SQVM};
#[derive(Clone)]
struct P {
	x: f64,
}
fn main() {
	let mut vm = SQVM::new(1024);
	let c = ClassBuilder::<P>::new("P").property("X", |p| p.x, |p, x| p.x = x).build(&mut vm);
	vm.register("mk", move |vm: &SQVM| c.instantiate(vm, P { x: 1.0 }));
	let t = std::time::Instant::now();
	vm.run_script("local p = mk(); local s = 0.0; for (local i = 0; i < 2000000; i++) s += p.X; ::r <- s;", "b").unwrap();
	println!("2M native property reads: {:?}", t.elapsed());
}
