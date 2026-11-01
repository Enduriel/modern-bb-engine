//! `Mouse` (a mouse event's `getID()`) and `MouseState` (its `getState()`).

use super::script_enum;

script_enum! {
	/// What a mouse event is about: a button, the pointer, or the wheel.
	Mouse {
		LeftButton = 1,
		RightButton = 2,
		MiddleButton = 3,
		ExtraButton1 = 4,
		ExtraButton2 = 5,
		Pointer = 6,
		Wheel = 7,
	}
}

script_enum! {
	/// What happened. A click acts on `Up`; every pointer move is `Moved`, button held
	/// or not.
	MouseState {
		Down = 0,
		Up = 1,
		Moved = 2,
		WheelUp = 3,
		WheelDown = 4,
	}
}
