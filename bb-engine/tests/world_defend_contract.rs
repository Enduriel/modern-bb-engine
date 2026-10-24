//! A defend-settlement-from-greenskins contract shows "The greenskins are in sight!" only
//! once a raiding party is within 700 of the town, not at once. The contract holds its
//! Home as a `WeakTableRef`, and `party.getDistanceTo(m.Home)` read 0 through one:
//! the popup came on the first update after accepting.
//! The original resolves the ref to the town (its natives read the argument's `CppInstance`,
//! which a ref's `_get` forwards); the same script runs there as a probe.

use std::path::Path;

use bb_engine::harness::{self, GameEngine};
use bb_engine::tactical_player as player;

fn eval(engine: &GameEngine, code: &str) -> String {
	player::eval_string(engine, code).unwrap_or_else(|e| panic!("{code}: {e}")).unwrap_or_default()
}

fn frames(engine: &GameEngine, n: usize) {
	for _ in 0..n {
		harness::run_frame(engine, 1000.0 / 60.0).expect("frame");
	}
}

#[test]
fn greenskins_are_in_sight_only_once_a_party_is_near() {
	let dir = Path::new(harness::STEAM_DATA_DIR);
	if !dir.exists() {
		harness::skip(format!("{} not present", dir.display()));
		return;
	}
	let engine = harness::open_install(dir).expect("open install");
	harness::boot_menu(&engine).expect("boot the menu");
	harness::prepare_new_campaign(&engine, "rust-engine-test").expect("prepare the campaign");
	harness::start_new_campaign(&engine).expect("startNewCampaign");
	engine.run_script("::World.State.m.IsAutosaving = false;", "no autosave").expect("autosave off");
	harness::unpause_world(&engine).expect("unpause");
	frames(&engine, 30);

	engine.run_script(include_str!("testdata/defend_greenskins.nut"), "defend_greenskins.nut").expect("the script");
	let taken = eval(&engine, "::DG.start()");
	eprintln!("{taken}");
	assert!(taken.contains("active=true"), "the contract is taken and active: {taken}");

	// The nearest party, measured to the town itself and through the contract's ref.
	let nearest = |engine: &GameEngine| -> (f64, f64) {
		let s = eval(
			engine,
			r#"(function() {
				local d = 1e9, r = 1e9;
				foreach (id in ::DG.Contract.m.UnitsSpawned)
				{
					local p = ::World.getEntityByID(id);
					if (p == null || !p.isAlive()) continue;
					d = ::Math.minf(d, p.getDistanceTo(::DG.Home));
					r = ::Math.minf(r, p.getDistanceTo(::DG.Contract.m.Home));
				}
				return d + " " + r;
			})()"#,
		);
		let mut v = s.split(' ').map(|x| x.parse::<f64>().unwrap_or_else(|_| panic!("distances: {s}")));
		(v.next().unwrap(), v.next().unwrap())
	};
	let (start, through_ref) = nearest(&engine);
	assert_eq!(start, through_ref, "getDistanceTo(m.Home) through the WeakTableRef");
	assert!(start > 700.0 + 200.0, "the chosen town's parties start well away: {start}");

	// Run until the popup's flag flips: by then a party is within 700 of the town (up to a
	// frame's travel before the contract's update ran), and the first seconds don't show it.
	let mut seconds = 0;
	loop {
		frames(&engine, 60);
		seconds += 1;
		eprintln!("{}", eval(&engine, "::DG.sample()"));
		let shown = eval(&engine, "::DG.Contract.m.Flags.get(\"IsEnemyHereDialogShown\") + \"\"") == "true";
		let (d, _) = nearest(&engine);
		if shown {
			assert!(seconds > 1, "shown at once");
			assert!(d <= 700.0 + 50.0, "shown with the nearest party {d} away");
			break;
		}
		assert!(d > 700.0 - 50.0 || seconds == 0, "a party {d} away and no popup after {seconds} s");
		assert!(seconds < 600, "no party came within 700 in 600 s (nearest {d})");
	}
}
