//! The sound listener: after `World.update()` or `Tactical.update()`, the output hears
//! from that scene's camera, at its rounded centre, with half its view's width in
//! world units (the screen's width times the zoom, halved) for the pan.

use std::cell::RefCell;
use std::rc::Rc;

use squirrel_bb::{AudioOut, Engine, LoadedScript, ScriptSource, VideoMode, Voice};

struct NoScripts;
impl ScriptSource for NoScripts {
	fn load(&self, _path: &str) -> Option<LoadedScript> {
		None
	}
	fn list_prefix(&self, _prefix: &str) -> Option<Vec<String>> {
		None
	}
}

type Heard = Rc<RefCell<Vec<((f32, f32), f32)>>>;

/// Plays nothing; notes where the listener goes.
struct ListenerLog(Heard);
impl AudioOut for ListenerLog {
	fn start(&mut self, _file: &str, _at: Option<(f32, f32)>) -> Option<Voice> {
		None
	}
	fn set_volume(&mut self, _voice: Voice, _volume: f32) {}
	fn set_pan(&mut self, _voice: Voice, _pan: f32) {}
	fn set_speed(&mut self, _voice: Voice, _speed: f32) {}
	fn set_paused(&mut self, _voice: Voice, _paused: bool) {}
	fn stop(&mut self, _voice: Voice) {}
	fn release(&mut self, _voice: Voice) {}
	fn position_ms(&self, _voice: Voice) -> u32 {
		0
	}
	fn stopped(&self, _voice: Voice) -> bool {
		true
	}
	fn length_ms(&mut self, _file: &str) -> u32 {
		0
	}
	fn set_listener(&mut self, at: (f32, f32), half_width: f32) {
		self.0.borrow_mut().push((at, half_width));
	}
	fn update(&mut self, _now_ms: u32) {}
}

#[test]
fn the_listener_is_the_updated_scenes_camera_and_view() {
	let engine = Engine::new(NoScripts);
	engine.video().set_mode(VideoMode {
		width: 2560,
		height: 1440,
		..VideoMode::DEFAULT
	});
	let heard = Heard::default();
	engine.audio().set_out(Box::new(ListenerLog(heard.clone())));
	engine
		.run_script(
			r#"
			local c = this.World.getCamera();
			c.setPos(1000.4, 800.6);
			c.Zoom = 2.0;
			this.World.update();
			local t = this.Tactical.getCamera();
			t.setPos(300, 200);
			t.Zoom = 1.5;
			this.Tactical.update();
			"#,
			"audio_listener",
		)
		.expect("the updates run");
	assert_eq!(*heard.borrow(), [((1000.0, 801.0), 2560.0), ((300.0, 200.0), 1920.0)]);
}
