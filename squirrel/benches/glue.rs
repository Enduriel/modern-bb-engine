//! The Rust↔Squirrel binding paths scripts hit hardest, each as a compiled Squirrel loop
//! of 100k operations: native property reads and writes (the `_get`/`_set` metamethods)
//! and method calls on a class shaped like `WorldTile` (~20 properties), and a call
//! through a Sqrat-style overload dispatcher, forwarding its arguments both ways the
//! crate offers (`forward_retained`: `Variadic<Retained>` + `call_member_forwarding`;
//! `forward_stack`: `Variadic<StackArg>` + `forward_member`). `bind_closures` rebinds a
//! class-level-sized table (40 methods, 10 data slots) to an instance, as every BB
//! object creation does for each level of its class chain. `with_payload` reads the
//! payloads of 1000 instances from the host, as the engine's per-frame passes over
//! sprites do.
//!
//! `cargo bench -p squirrel --bench glue -- --save-baseline before`, change the code,
//! then `-- --baseline before`.

use std::hint::black_box;
use std::time::Duration;

use criterion::{Criterion, criterion_group, criterion_main};
use squirrel::{ClassBuilder, Retained, SQVM, SqTable, StackArg, This, Variadic};

#[derive(Clone, Default)]
struct Tile {
	fields: [i64; 16],
	x: f64,
	y: f64,
	flag: bool,
}

fn vm_with_tile() -> SQVM {
	let mut vm = SQVM::new(1024);
	let mut b = ClassBuilder::<Tile>::new("Tile");
	for (i, name) in [
		"Type",
		"TacticalType",
		"Subregion",
		"Region",
		"RoadDirections",
		"RiverDirections",
		"HeatFromSettlements",
		"Level",
		"Overlay",
		"Detail",
		"Visited",
		"Owner",
		"Danger",
		"Value",
		"Weight",
		"Id",
	]
	.into_iter()
	.enumerate()
	{
		b = b.property(
			name,
			move |t: &Tile| t.fields[i],
			move |t: &mut Tile, v: i64| t.fields[i] = v,
		);
	}
	let class = b
		.property("X", |t: &Tile| t.x, |t: &mut Tile, v: f64| t.x = v)
		.property("Y", |t: &Tile| t.y, |t: &mut Tile, v: f64| t.y = v)
		.property(
			"IsOccupied",
			|t: &Tile| t.flag,
			|t: &mut Tile, v: bool| t.flag = v,
		)
		.method("sum", |t: &mut Tile| t.x + t.y)
		.build(&mut vm);
	let tile = class.instantiate(&vm, Tile::default());
	SqTable::root(&vm).set("T", tile);

	// Two overloaded `add(a, b)` dispatchers over the same `__sqrat_ol_ add_2` leaf.
	let retained = vm.make_closure(|vm: &SQVM, this: This, args: Variadic<Retained>| {
		let forwarded: Vec<_> = args.0.into_iter().map(|r| r.0).collect();
		let key = format!("__sqrat_ol_ add_{}", forwarded.len());
		this.call_member_forwarding(vm, &key, &forwarded)?
			.ok_or("no overload".to_string())
	});
	let stack = vm.make_closure(|vm: &SQVM, this: This, args: Variadic<StackArg>| {
		let key = format!("__sqrat_ol_ add_{}", args.0.len());
		this.forward_member(vm, key.as_str(), args.0.as_slice())?
			.ok_or("no overload".to_string())
	});
	let leaf = vm.make_closure(|a: i64, b: i64| a + b);
	let mut ops = SqTable::new(&vm);
	ops.set("__sqrat_ol_ add_2", leaf);
	ops.set("add_retained", retained);
	ops.set("add_stack", stack);
	SqTable::root(&vm).set("Ops", ops);
	vm
}

fn bench(c: &mut Criterion) {
	let vm = vm_with_tile();
	let cases = [
		(
			"property_get",
			"local t = ::T, s = 0; for (local i = 0; i < 100000; i++) s += t.Region; return s;",
		),
		(
			"property_set",
			"local t = ::T; for (local i = 0; i < 100000; i++) t.Region = i;",
		),
		(
			"method_call",
			"local t = ::T, s = 0.0; for (local i = 0; i < 100000; i++) s += t.sum(); return s;",
		),
		(
			"forward_retained",
			"local o = ::Ops, s = 0; for (local i = 0; i < 100000; i++) s += o.add_retained(i, 1); return s;",
		),
		(
			"forward_stack",
			"local o = ::Ops, s = 0; for (local i = 0; i < 100000; i++) s += o.add_stack(i, 1); return s;",
		),
	];
	let mut group = c.benchmark_group("glue");
	group
		.sample_size(20)
		.measurement_time(Duration::from_secs(5));
	for (name, src) in cases {
		let script = vm.compile(src, name).expect("compile");
		group.bench_function(name, |b| b.iter(|| script.call(&vm, ()).expect("run")));
	}
	let level = vm
		.compile(
			"local t = {}; for (local i = 0; i < 40; i++) t[\"m\" + i] <- function() { return i; };
			 for (local i = 0; i < 10; i++) t[\"d\" + i] <- i; return t;",
			"level",
		)
		.unwrap()
		.call_ret::<squirrel::SqStrongRef, _>(&vm, ())
		.unwrap()
		.unwrap();
	let mut level = level.as_table(&vm).unwrap();
	let env = SqTable::new(&vm);
	group.bench_function("bind_closures", |b| {
		b.iter(|| level.bind_closures_to(black_box(&env)))
	});
	let mut vm = SQVM::new(1024);
	let class = ClassBuilder::<Tile>::new("Payload").build(&mut vm);
	let handles: Vec<squirrel::SqStrongRef> = (0..1000).map(|_| class.instantiate(&vm, Tile::default())).collect();
	group.bench_function("with_payload", |b| {
		b.iter(|| {
			let mut s = 0.0;
			for h in &handles {
				s += class.with_payload(&vm, h, |t| t.x).unwrap();
			}
			black_box(s)
		})
	});
	group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
