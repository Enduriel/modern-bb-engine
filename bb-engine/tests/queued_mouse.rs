//! A mouse event the host queues (`Engine::queue_mouse`, as `bb` queues the pointer's
//! last move of a frame) reaches the states at the next frame, in order, and not before:
//! at the main menu the state puts the cursor where each move says (`Cursor.setPosition`
//! from `main_menu_state.onMouseInput`).
//!
//! Skips without the Steam install.

use std::path::Path;

use bb_engine::{harness, tactical_player as player};
use squirrel_bb::MouseInput;

const MENU: &str = "::RootState.get(\"MainMenuState\").m.MainMenuScreen";

#[test]
fn queued_mouse_events_reach_the_states_at_the_frame_in_order() {
	let dir = Path::new(harness::STEAM_DATA_DIR);
	if !dir.exists() {
		harness::skip(format!("{} not present", dir.display()));
		return;
	}
	let mut engine = harness::open_install(dir).expect("open install");
	engine.vm_mut().install_standard_error_handlers();
	harness::boot_menu(&engine).expect("boot the menu");
	for _ in 0..120 {
		let shown = player::eval_i64(&engine, &format!("({MENU}.isVisible() && !{MENU}.isAnimating()) ? 1 : 0")).expect("menu");
		if shown == Some(1) {
			break;
		}
		harness::run_frame(&engine, 1000.0 / 60.0).expect("frame");
	}
	let at = |engine: &harness::GameEngine| engine.cursor().1;
	let before = at(&engine);
	engine.queue_mouse(MouseInput::moved(123, 45));
	engine.queue_mouse(MouseInput::moved(321, 54));
	assert_eq!(at(&engine), before, "a queued move reached the states before the frame");
	harness::run_frame(&engine, 1000.0 / 60.0).expect("frame");
	assert_eq!(at(&engine), (321, 54), "the frame's last move is where the cursor is");
	harness::run_frame(&engine, 1000.0 / 60.0).expect("frame");
	assert_eq!(at(&engine), (321, 54), "a queued event is dispatched once");
}
