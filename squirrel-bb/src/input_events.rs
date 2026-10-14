//! The input events the engine hands the states: `onMouseInput(mouse)` and
//! `onKeyInput(key)` receive small native objects the scripts only read.
//!
//! The codes are the ones the vanilla scripts test for. Mouse `getID()`: 1 the left
//! button, 2 the right, 6 a move, 7 the wheel. Mouse `getState()`: 0 pressed, 1 released
//! (a click acts on the release), 2 moved with a button down, 3 and 4 the wheel turned.
//! Keys: `getKey()` is the engine's key code (e.g. 41 is Escape), `getState()` 0 or 1,
//! `getModifier()` the held modifier.

use squirrel::{ClassBuilder, SQVM, SqClass, SqStrongRef};

/// A mouse event: what button or motion, its state, and the cursor's screen position.
#[derive(Clone, Copy, Debug)]
pub struct MouseInput {
	pub id: i64,
	pub state: i64,
	pub x: i64,
	pub y: i64,
}

impl MouseInput {
	/// The cursor moved to `(x, y)`.
	pub fn moved(x: i64, y: i64) -> Self {
		MouseInput { id: 6, state: 0, x, y }
	}

	/// The left button went down at `(x, y)`.
	pub fn left_down(x: i64, y: i64) -> Self {
		MouseInput { id: 1, state: 0, x, y }
	}

	/// The left button came up at `(x, y)` (what acts as a click).
	pub fn left_up(x: i64, y: i64) -> Self {
		MouseInput { id: 1, state: 1, x, y }
	}

	/// The right button came up at `(x, y)` (cancels the current action).
	pub fn right_up(x: i64, y: i64) -> Self {
		MouseInput { id: 2, state: 1, x, y }
	}
}

/// A key event.
#[derive(Clone, Copy, Debug)]
pub struct KeyInput {
	pub key: i64,
	pub state: i64,
	pub modifier: i64,
}

/// The two event classes, built once.
pub(crate) struct InputClasses {
	mouse: SqClass<MouseInput>,
	key: SqClass<KeyInput>,
}

impl InputClasses {
	pub(crate) fn build(vm: &mut SQVM) -> Self {
		let mouse = ClassBuilder::<MouseInput>::new("MouseEvent")
			.method("getID", |e: &mut MouseInput| e.id)
			.method("getState", |e: &mut MouseInput| e.state)
			.method("getX", |e: &mut MouseInput| e.x)
			.method("getY", |e: &mut MouseInput| e.y)
			.build(vm);
		let key = ClassBuilder::<KeyInput>::new("KeyEvent")
			.method("getKey", |e: &mut KeyInput| e.key)
			.method("getState", |e: &mut KeyInput| e.state)
			.method("getModifier", |e: &mut KeyInput| e.modifier)
			.build(vm);
		InputClasses { mouse, key }
	}

	pub(crate) fn mouse(&self, vm: &SQVM, input: MouseInput) -> SqStrongRef {
		self.mouse.instantiate(vm, input)
	}

	pub(crate) fn key(&self, vm: &SQVM, input: KeyInput) -> SqStrongRef {
		self.key.instantiate(vm, input)
	}
}
