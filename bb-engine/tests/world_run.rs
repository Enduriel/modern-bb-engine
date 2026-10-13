//! End-to-end: a new campaign keeps running headless. After the boot, the world is
//! unpaused and driven frame by frame through the real loop (scheduled events, state
//! updates with entity thinkers, the background world simulation) with the headless UI
//! client and the scripted player answering events. Checks that a game minute passes
//! without a script error and that the world is alive: the clock advances, parties
//! move, and the population changes.
//!
//! Skips if no Steam install. Slow in debug builds: it runs all of worldgen first.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use bb_engine::harness;

/// Reports the clock and every party's position as `OUT ...` lines.
const REPORT: &str = r#"
local t = ::World.getTime();
::print("OUT time " + t.Time + "\n");
foreach (e in ::World.getAllEntitiesAtPos(::World.State.getPlayer().getPos(), 100000.0))
	if (e.isParty()) ::print("OUT party " + e.getID() + " " + e.getPos().X + "," + e.getPos().Y + "\n");
"#;

/// `(time, [(id, pos)])` from a [`REPORT`] run.
fn snapshot(engine: &harness::GameEngine, out: &Rc<RefCell<String>>) -> (f64, Vec<(String, String)>) {
	out.borrow_mut().clear();
	engine.run_script(REPORT, "report").expect("report");
	let text = out.borrow();
	let mut time = 0.0;
	let mut parties = Vec::new();
	for line in text.lines().filter_map(|l| l.strip_prefix("OUT ")) {
		if let Some(t) = line.strip_prefix("time ") {
			time = t.parse().expect("time");
		} else if let Some(p) = line.strip_prefix("party ") {
			let (id, pos) = p.split_once(' ').expect("party line");
			parties.push((id.to_string(), pos.to_string()));
		}
	}
	(time, parties)
}

#[test]
fn a_new_campaign_runs_for_a_game_minute() {
	let dir = Path::new(harness::STEAM_DATA_DIR);
	if !dir.exists() {
		eprintln!("skipping: {} not present", dir.display());
		return;
	}

	let mut engine = harness::open_install(dir).expect("open install");
	let out: Rc<RefCell<String>> = Rc::default();
	let sink = out.clone();
	engine.vm_mut().set_print_handler(move |s| sink.borrow_mut().push_str(s));

	harness::boot_menu(&engine).expect("boot the menu");
	harness::prepare_new_campaign(&engine, "rust-engine-test").expect("prepare the campaign");
	harness::start_new_campaign(&engine).expect("startNewCampaign");
	let (start_time, start_parties) = snapshot(&engine, &out);

	harness::unpause_world(&engine).expect("unpause");
	for frame in 0..3600 {
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap_or_else(|e| panic!("frame {frame}: {e}"));
	}
	let (end_time, end_parties) = snapshot(&engine, &out);

	// A real minute at 1x is a game minute; events pause the world only for the frame
	// they are answered in.
	assert!(end_time - start_time > 55.0, "clock advanced {}", end_time - start_time);
	// Parties that were there at the start and still are have moved.
	let moved = start_parties
		.iter()
		.filter(|(id, pos)| end_parties.iter().any(|(i, p)| i == id && p != pos))
		.count();
	assert!(moved > 0, "no party moved");
	// The world spawned new parties along the way.
	assert!(
		end_parties.iter().any(|(id, _)| !start_parties.iter().any(|(i, _)| i == id)),
		"no new parties"
	);
}
