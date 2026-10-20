//! The game's mouse cursor (the original's `UICursor`, `0x521D90`, and its `UI`
//! natives; disassembled). The cursors are sprites in `ui.brush`
//! (`Const.UI.Cursor`: `cursor_hand`, `cursor_attack`, …), which the original draws
//! last, over the UI, at the position the scripts last set (`UI.setCursorPosition`,
//! which the states call on every mouse move), with no scaling. The window's own
//! cursor is hidden. With the hardware cursor on (`Settings.getControlSettings()
//! .UseHardwareCursor`, off by default), the original
//! shows the system cursor from `<install>/cursors/<name>.cur` instead.
//!
//! - `setCursor(name)`: the sprite to draw; an unknown name draws nothing. Setting it
//!   restarts its animation (the attack cursor's ten frames), on virtual time.
//! - `setCursorHardware(on)` / `isCursorHardware()`: true until the scripts set it.
//! - `getCursorSize()`: `{W, H}`, the sprite's size; `getCursorOffsets()`: `{X, Y}`,
//!   its offset (`X`, and `-Y` of the brush's y-up offset); `{0, 0}` without one.
//!
//! The UI's CSS cursors change nothing (the original ignores its UI's cursor changes).

use std::cell::RefCell;
use std::rc::Rc;

use squirrel::{SQVM, SqStrongRef, SqTable};

use super::Sprites;
use super::sprite_color::virtual_time;

/// What the cursor looks like now, for the host to draw.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CursorLook {
	/// The sprite (`None`: nothing to draw).
	pub brush: Option<String>,
	/// The hardware cursor is on: the host shows the system cursor `cursors/<name>.cur`.
	pub hardware: bool,
	/// The name the scripts set (the hardware cursor's file).
	pub name: String,
	/// Virtual seconds when it was set (its animation starts there).
	pub since: f32,
}

struct CursorState {
	look: CursorLook,
}

/// The cursor's state, shared by its natives and the host.
#[derive(Clone)]
pub struct UiCursor(Rc<RefCell<CursorState>>);

impl Default for UiCursor {
	fn default() -> Self {
		UiCursor(Rc::new(RefCell::new(CursorState {
			look: CursorLook {
				hardware: true,
				..CursorLook::default()
			},
		})))
	}
}

impl UiCursor {
	/// What the cursor looks like now.
	pub fn look(&self) -> CursorLook {
		self.0.borrow().look.clone()
	}

	/// Replace the `UI` table's cursor natives (after the UI bridge installs them).
	pub fn install(&self, vm: &mut SQVM, sprites: &Sprites) {
		let set_cursor = {
			let (state, sprites) = (self.0.clone(), sprites.clone());
			vm.make_closure(move |vm: &SQVM, name: String| {
				if name.is_empty() {
					return;
				}
				let known = sprites.with(|r| r.get(&name).is_some());
				if !known {
					crate::html_log::engine(
						crate::html_log::Class::Warning,
						"SceneManager",
						&format!("Unknown Brush requested: {name}"),
					);
				}
				let since = virtual_time(vm);
				let mut s = state.borrow_mut();
				s.look.brush = known.then(|| name.clone());
				s.look.name = name;
				s.look.since = since;
			})
		};
		let set_hardware = {
			let state = self.0.clone();
			vm.make_closure(move |on: bool| state.borrow_mut().look.hardware = on)
		};
		let is_hardware = {
			let state = self.0.clone();
			vm.make_closure(move || state.borrow().look.hardware)
		};
		// The current sprite's size and offset (`0x53A1A0`, `0x53A250`).
		let sprite_of = {
			let (state, sprites) = (self.0.clone(), sprites.clone());
			move || -> Option<(i64, i64, i64, i64)> {
				let brush = state.borrow().look.brush.clone()?;
				sprites.with(|r| {
					r.get(&brush).map(|(_, s)| {
						(i64::from(s.width), i64::from(s.height), i64::from(s.offset_x), -i64::from(s.offset_y))
					})
				})
			}
		};
		let get_size = {
			let sprite_of = sprite_of.clone();
			vm.make_closure(move |vm: &SQVM| -> SqStrongRef {
				let (w, h, _, _) = sprite_of().unwrap_or_default();
				let mut t = SqTable::new(vm);
				t.set("W", w);
				t.set("H", h);
				t.to_strong()
			})
		};
		let get_offsets = vm.make_closure(move |vm: &SQVM| -> SqStrongRef {
			let (_, _, x, y) = sprite_of().unwrap_or_default();
			let mut t = SqTable::new(vm);
			t.set("X", x);
			t.set("Y", y);
			t.to_strong()
		});
		if let Some(mut ui) = SqTable::root(vm).get::<SqTable>("UI") {
			ui.set("setCursor", set_cursor);
			ui.set("setCursorHardware", set_hardware);
			ui.set("isCursorHardware", is_hardware);
			ui.set("getCursorSize", get_size);
			ui.set("getCursorOffsets", get_offsets);
		}
	}
}
