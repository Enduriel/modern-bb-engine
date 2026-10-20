//! The input events the engine hands the states: `onMouseInput(mouse)` and
//! `onKeyInput(key)` receive small native objects the scripts only read.
//!
//! The codes are the original's, from the enums it binds (disassembled): `Mouse`
//! (`getID()`): 1 `LeftButton`, 2 `RightButton`, 3 `MiddleButton`, 4/5 `ExtraButton1`/
//! `2`, 6 `Pointer` (a move), 7 `Wheel`. `MouseState` (`getState()`): 0 `Down`, 1 `Up`
//! (a click acts on the release), 2 `Moved` (every pointer move, button held or not),
//! 3 `WheelUp`, 4 `WheelDown`; and `getModifier()`, the modifier keys held.
//! Keys: `getKey()` is the engine's key code (e.g. 41 is Escape); `KeyState`
//! (`getState()`): 0 `Up`, 1 `Down`, so vanilla's handlers, which test for 0, act when a
//! key is released and scroll while one is held (a held key comes every frame, see
//! `key_queue`); `KeyModifier` (`getModifier()`), a mask: 1 `Shift`, 2 `Control`, 4
//! `Alt`; `getFire()`, which the original fills with the same as the state.

use squirrel::{ClassBuilder, SQVM, SqClass, SqStrongRef};

/// A mouse event: what button or motion, its state, and the cursor's screen position.
#[derive(Clone, Copy, Debug)]
pub struct MouseInput {
	pub id: i64,
	pub state: i64,
	pub x: i64,
	pub y: i64,
	/// The modifier keys held (`KeyModifier` mask).
	pub modifier: i64,
}

impl MouseInput {
	/// The cursor moved to `(x, y)`.
	pub fn moved(x: i64, y: i64) -> Self {
		MouseInput { id: 6, state: 2, x, y, modifier: 0 }
	}

	/// The left button went down at `(x, y)`.
	pub fn left_down(x: i64, y: i64) -> Self {
		MouseInput { id: 1, state: 0, x, y, modifier: 0 }
	}

	/// The left button came up at `(x, y)` (what acts as a click).
	pub fn left_up(x: i64, y: i64) -> Self {
		MouseInput { id: 1, state: 1, x, y, modifier: 0 }
	}

	/// The right button came up at `(x, y)` (cancels the current action).
	pub fn right_up(x: i64, y: i64) -> Self {
		MouseInput { id: 2, state: 1, x, y, modifier: 0 }
	}
}

/// A key event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyInput {
	pub key: i64,
	pub state: i64,
	pub modifier: i64,
	pub fire: i64,
}

impl KeyInput {
	/// Key `key` went down (`KeyState.Down`), no modifier held.
	pub fn pressed(key: i64) -> Self {
		KeyInput { key, state: 1, modifier: 0, fire: 1 }
	}

	/// Key `key` came up (`KeyState.Up`), which is when vanilla's key handlers act.
	pub fn released(key: i64) -> Self {
		KeyInput { key, state: 0, modifier: 0, fire: 0 }
	}
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
			.method("getModifier", |e: &mut MouseInput| e.modifier)
			.build(vm);
		let key = ClassBuilder::<KeyInput>::new("KeyEvent")
			.method("getKey", |e: &mut KeyInput| e.key)
			.method("getState", |e: &mut KeyInput| e.state)
			.method("getModifier", |e: &mut KeyInput| e.modifier)
			.method("getFire", |e: &mut KeyInput| e.fire)
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
