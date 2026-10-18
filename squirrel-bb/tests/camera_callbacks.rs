//! The world camera's movement callbacks, as the camera class's (the battle camera has
//! the same): `update()` calls each registered `start` function once the camera has
//! begun to move, and each `end` function once it has stopped.

use squirrel::{SQVM, SqTable};
use squirrel_bb::install_natives;

#[test]
fn update_calls_start_as_the_camera_moves_and_end_as_it_stops() {
	let mut vm = SQVM::new(1 << 16);
	install_natives(&mut vm);
	vm.run_script(
		r#"
		::log <- "";
		local start = function() { ::log += "start "; };
		local end = function() { ::log += "end "; };
		local c = this.World.getCamera();
		c.registerCallbacks(start, end);
		c.update();                          // still: nothing
		c.moveToPos(this.createVec(4000, 3000));
		c.update();                          // begins to move
		c.update();                          // still moving: nothing
		c.cancelMovement();
		c.update();                          // stopped
		c.unregisterCallbacks(start, end);
		c.moveToPos(this.createVec(1000, 1000));
		c.update();                          // unregistered: nothing
		"#,
		"camera_callbacks",
	)
	.expect("the callbacks run");
	assert_eq!(SqTable::root(&vm).get::<String>("log").as_deref(), Some("start end "));
}
