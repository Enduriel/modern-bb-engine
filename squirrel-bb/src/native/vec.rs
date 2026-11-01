//! The `vec2f` / `vec2i` native classes and the `createVec` / `getVecDistance` /
//! `getVecDir` root functions.
//!
//! `vec2f` is the workhorse 2D float vector (sprite offsets, world positions,
//! particle coordinates, directions); scripts read and mutate `.X`/`.Y` and pass
//! instances to native engine methods. `vec2i` is the integer vector of tile
//! coordinates: a tile's `Coords`/`SquareCoords`, `getMapSize()`, `worldToTile()`
//! (probed 2026-11-01). Instances come from the engine; natives that take tile
//! coordinates take only a `vec2i` (see [`tile_coords`]).

use squirrel::{ClassBuilder, Native, Number, SQVM, SqClass, SqStrongRef};

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

/// The `vec2f` class, for natives that hand back a vector (as `createVec` makes one).
pub(crate) type VecClass = SqClass<Vec2f>;

/// The `vec2i` class, for natives that hand back tile coordinates.
pub(crate) type Vec2iClass = SqClass<Vec2i>;

/// The tile coordinates a native was handed: a `vec2i`'s. The original reads any
/// instance's payload as two i32s unchecked, so a `vec2f` names the tile its float bits
/// spell and a table crashes it (probed 2026-11-01); nothing in vanilla passes either.
/// Here anything but a `vec2i` is a script error.
pub(crate) fn tile_coords(vm: &SQVM, v: &SqStrongRef, native: &str) -> Result<(i64, i64), String> {
	if let Some(xy) = v.with_native(vm, |p: &mut Vec2i| (p.x, p.y)) {
		return Ok(xy);
	}
	let got = if v.with_native(vm, |_: &mut Vec2f| ()).is_some() {
		" (got a vec2f, a world position: World.worldToTile turns one into tile coordinates)"
	} else {
		""
	};
	Err(format!("{native}: expected tile coordinates, a vec2i like tile.Coords{got}"))
}

/// The position (or offset, or direction) a native was handed: a `vec2f`'s. As with
/// tile coordinates, the original reads any instance's payload unchecked (a `vec2i` as
/// two floats of its integers' bits; a table crashes it), and vanilla passes only
/// `vec2f`s (`createVec`, `tile.Pos`, `getPos()`). Here anything else is a script error.
pub(crate) fn position(vm: &SQVM, v: &SqStrongRef, native: &str) -> Result<(f64, f64), String> {
	if let Some(xy) = v.with_native(vm, |p: &mut Vec2f| (p.x, p.y)) {
		return Ok(xy);
	}
	let got = if v.with_native(vm, |_: &mut Vec2i| ()).is_some() {
		" (got a vec2i, tile coordinates: World.tileToWorld turns them into a position)"
	} else {
		""
	};
	Err(format!("{native}: expected a vector, a vec2f like createVec makes{got}"))
}

/// [`position`] in f32, as most of the engine keeps positions.
pub(crate) fn position_f32(vm: &SQVM, v: &SqStrongRef, native: &str) -> Result<(f32, f32), String> {
	position(vm, v, native).map(|(x, y)| (x as f32, y as f32))
}

/// Register `vec2f`/`vec2i` and the vector root functions; their classes.
pub(crate) fn install(vm: &mut SQVM) -> (VecClass, Vec2iClass) {
	let vec2f = ClassBuilder::<Vec2f>::new("vec2f")
		.property("X", |v| v.x, |v, x| v.x = x)
		.property("Y", |v| v.y, |v, y| v.y = y)
		.build(vm);

	// Tile coordinates; no script constructs one directly. Written values truncate to
	// integers, as the original's (`c.X = 1.7` stores 1).
	let vec2i = ClassBuilder::<Vec2i>::new("vec2i")
		.property("X", |v| v.x, |v, x| v.x = x)
		.property("Y", |v| v.y, |v, y| v.y = y)
		.build(vm);

	// createVec(x, y) -> vec2f. Always float; integer args are coerced.
	let factory = vec2f.clone();
	vm.register("createVec", move |x: Number, y: Number| factory.new_instance(Vec2f { x: x.0, y: y.0 }));

	// getVecDistance(a, b) -> Euclidean distance.
	vm.register("getVecDistance", |a: Native<Vec2f>, b: Native<Vec2f>| {
		let dx = a.0.x - b.0.x;
		let dy = a.0.y - b.0.y;
		(dx * dx + dy * dy).sqrt()
	});

	// getVecDir(a, b) -> unit vector pointing from a toward b (zero vector if the
	// points coincide).
	let dir_factory = vec2f.clone();
	vm.register(
		"getVecDir",
		move |a: Native<Vec2f>, b: Native<Vec2f>| {
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
			dir_factory.new_instance(dir)
		},
	);
	(vec2f, vec2i)
}
