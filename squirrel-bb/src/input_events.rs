//! The input events the engine hands the states: `onMouseInput(mouse)` and
//! `onKeyInput(key)` receive small native objects the scripts only read.
//!
//! The codes are the original's enums ([`crate::enums`]): a mouse event's `getID()` is
//! a [`Mouse`] and its `getState()` a [`MouseState`]; a key event's `getKey()` is a
//! [`Key`], its `getState()` a [`KeyState`] and its `getFire()` a [`KeyFire`]. Both
//! carry the [`KeyModifier`] mask held, which the host may give the Command key for the
//! UI page; the scripts never see it.

use squirrel::{ClassBuilder, SQVM, SqClass, SqStrongRef};

use crate::enums::{Key, KeyFire, KeyModifier, KeyState, Mouse, MouseState};

/// A mouse event: what button or motion, its state, and the cursor's screen position.
#[derive(Clone, Copy, Debug)]
pub struct MouseInput {
	pub id: Mouse,
	pub state: MouseState,
	pub x: i64,
	pub y: i64,
	/// The modifier keys held.
	pub modifier: KeyModifier,
}

impl MouseInput {
	/// The cursor moved to `(x, y)`.
	pub fn moved(x: i64, y: i64) -> Self {
		MouseInput { id: Mouse::Pointer, state: MouseState::Moved, x, y, modifier: KeyModifier::NONE }
	}

	/// The left button went down at `(x, y)`.
	pub fn left_down(x: i64, y: i64) -> Self {
		MouseInput { id: Mouse::LeftButton, state: MouseState::Down, x, y, modifier: KeyModifier::NONE }
	}

	/// The left button came up at `(x, y)` (what acts as a click).
	pub fn left_up(x: i64, y: i64) -> Self {
		MouseInput { id: Mouse::LeftButton, state: MouseState::Up, x, y, modifier: KeyModifier::NONE }
	}

	/// The right button came up at `(x, y)` (cancels the current action).
	pub fn right_up(x: i64, y: i64) -> Self {
		MouseInput { id: Mouse::RightButton, state: MouseState::Up, x, y, modifier: KeyModifier::NONE }
	}
}

/// A key event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyInput {
	pub key: Key,
	pub state: KeyState,
	pub modifier: KeyModifier,
	pub fire: KeyFire,
}

impl KeyInput {
	/// Key `key` went down, no modifier held.
	pub fn pressed(key: Key) -> Self {
		KeyInput { key, state: KeyState::Down, modifier: KeyModifier::NONE, fire: KeyFire::Repeated }
	}

	/// Key `key` came up, which is when vanilla's key handlers act.
	pub fn released(key: Key) -> Self {
		KeyInput { key, state: KeyState::Up, modifier: KeyModifier::NONE, fire: KeyFire::Once }
	}

	/// Key `key` went down (`down`) or up, with `modifier` held. `getFire()` is the
	/// state, as the original fills it.
	pub fn new(key: Key, down: bool, modifier: KeyModifier) -> Self {
		let (state, fire) = if down { (KeyState::Down, KeyFire::Repeated) } else { (KeyState::Up, KeyFire::Once) };
		KeyInput { key, state, modifier, fire }
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
			.method("getID", |e: &mut MouseInput| e.id.code())
			.method("getState", |e: &mut MouseInput| e.state.code())
			.method("getX", |e: &mut MouseInput| e.x)
			.method("getY", |e: &mut MouseInput| e.y)
			.method("getModifier", |e: &mut MouseInput| e.modifier.for_scripts())
			.build(vm);
		let key = ClassBuilder::<KeyInput>::new("KeyEvent")
			.method("getKey", |e: &mut KeyInput| e.key.code())
			.method("getState", |e: &mut KeyInput| e.state.code())
			.method("getModifier", |e: &mut KeyInput| e.modifier.for_scripts())
			.method("getFire", |e: &mut KeyInput| e.fire.code())
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
