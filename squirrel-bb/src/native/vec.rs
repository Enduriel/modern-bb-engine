//! The `vec2f` / `vec2i` native classes and the `createVec` / `getVecDistance` /
//! `getVecDir` root functions.
//!
//! `vec2f` is the workhorse 2D float vector (sprite offsets, world positions,
//! particle coordinates, directions); scripts read and mutate `.X`/`.Y` and pass
//! instances to native engine methods. `vec2i` is the integer vector returned by
//! `getMapSize()` (registered here so it exists; instances come from the engine).

use squirrel::{ClassBuilder, Native, Number, SQVM};

/// A 2D float vector — the payload behind a `vec2f` instance.
#[derive(Clone, Copy)]
pub struct Vec2f {
	pub x: f64,
	pub y: f64,
}

/// A 2D integer vector — the payload behind a `vec2i` instance.
#[derive(Clone, Copy)]
pub struct Vec2i {
	pub x: i64,
	pub y: i64,
}

/// Register `vec2f`/`vec2i` and the vector root functions.
pub(crate) fn install(vm: &mut SQVM) {
	let vec2f = ClassBuilder::<Vec2f>::new("vec2f")
		.property("X", |v| v.x, |v, x| v.x = x)
		.property("Y", |v| v.y, |v, y| v.y = y)
		.build(vm);

	// vec2i exists for `instanceof` and as the type the engine hands back from
	// getMapSize(); no script ever constructs one directly.
	ClassBuilder::<Vec2i>::new("vec2i")
		.property("X", |v| v.x, |v, x| v.x = x)
		.property("Y", |v| v.y, |v, y| v.y = y)
		.build(vm);

	// createVec(x, y) -> vec2f. Always float; integer args are coerced.
	let factory = vec2f.clone();
	vm.register("createVec", move |vm: &SQVM, x: Number, y: Number| {
		factory.instantiate(vm, Vec2f { x: x.0, y: y.0 })
	});

	// getVecDistance(a, b) -> Euclidean distance.
	vm.register("getVecDistance", |a: Native<Vec2f>, b: Native<Vec2f>| {
		let dx = a.0.x - b.0.x;
		let dy = a.0.y - b.0.y;
		(dx * dx + dy * dy).sqrt()
	});

	// getVecDir(a, b) -> unit vector pointing from a toward b (zero vector if the
	// points coincide).
	let dir_factory = vec2f;
	vm.register(
		"getVecDir",
		move |vm: &SQVM, a: Native<Vec2f>, b: Native<Vec2f>| {
			let dx = b.0.x - a.0.x;
			let dy = b.0.y - a.0.y;
			let len = (dx * dx + dy * dy).sqrt();
			let dir = if len == 0.0 {
				Vec2f { x: 0.0, y: 0.0 }
			} else {
				Vec2f {
					x: dx / len,
					y: dy / len,
				}
			};
			dir_factory.instantiate(vm, dir)
		},
	);
}
