//! A campaign doesn't grow with the days played: world entities die all the time (parties
//! destroyed, locations razed, footprints of the map), and each dead one lets go of its
//! script object, its sprites and its labels; the slots of the dead are taken by new
//! entities once nothing holds their script objects. Over days of play without battles,
//! the number of entity slots and the heap stay level.
//!
//! Before this, a dead entity's record kept its whole script graph until the next
//! `clearScene` or load: ~35 records and ~2.7 MB a day.
//!
//! Skips without the Steam install.

use std::path::Path;

use bb_engine::{harness, heap, tactical_player as player};

fn heap_mb() -> f64 {
	heap::bytes_in_use() as f64 / 1_048_576.0
}

#[test]
fn days_of_play_dont_grow_the_world() {
	let dir = Path::new(harness::STEAM_DATA_DIR);
	if !dir.exists() {
		eprintln!("skipping: {} not present", dir.display());
		return;
	}
	let mut engine = harness::open_install(dir).expect("open install");
	engine.vm_mut().install_standard_error_handlers();
	harness::boot_menu(&engine).expect("boot the menu");
	harness::prepare_new_campaign(&engine, "rust-engine-test").expect("prepare the campaign");
	harness::start_new_campaign(&engine).expect("startNewCampaign");
	harness::unpause_world(&engine).expect("unpause");
	// No battles: the player's party can't be attacked; the world runs at 4x.
	engine
		.run_script(
			"local p = ::World.State.getPlayer(); p.m.IsAttackable = false; p.m.IsAttackableByAI = false; ::World.setSpeedMult(4.0);",
			"no battles",
		)
		.expect("setup");
	let day = |e: &harness::GameEngine| player::eval_i64(e, "::World.getTime().Days").unwrap().unwrap_or(0);
	let alive = |e: &harness::GameEngine| player::eval_i64(e, "::World.getAllEntitiesAtPos(::createVec(0, 0), 10000000.0).len()").unwrap().unwrap_or(0);
	let mut samples = Vec::new();
	let mut last = day(&engine);
	let mut frames = 0u64;
	while last < 12 {
		harness::run_frame(&engine, 1000.0 / 60.0).expect("frame");
		frames += 1;
		if frames % 60 == 0 {
			// Keep the world running through menus and pauses, as a player would.
			engine
				.run_script("if (::World.State.isPaused()) { ::World.State.setAutoPause(false); ::World.State.setPause(false); } ::World.setSpeedMult(4.0);", "run")
				.expect("run");
		}
		let d = day(&engine);
		if d != last {
			last = d;
			let slots = engine.world_entities().count();
			let (live, heap) = (alive(&engine), heap_mb());
			eprintln!("day {d}: {slots} entity slots, {live} entities the scripts see, heap {heap:.1} MB");
			samples.push((d, slots, heap));
		}
	}
	let (first, last) = (samples[1], *samples.last().unwrap());
	// Entities come and go with the days, but their slots don't pile up.
	assert!(last.1 < first.1 + first.1 / 5, "entity slots grew from {} on day {} to {} on day {}", first.1, first.0, last.1, last.0);
	if first.2 > 0.0 {
		let per_day = (last.2 - first.2) / (last.0 - first.0) as f64;
		assert!(per_day < 1.0, "the heap grew {per_day:.2} MB a day ({:.1} -> {:.1})", first.2, last.2);
	}
}
