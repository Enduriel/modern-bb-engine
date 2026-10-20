//! Tests for the `Time` service: the three clocks, `scheduleEvent` firing, and
//! the host-driven `tick`/`setRound` advancement.

use squirrel::SQVM;
use squirrel_bb::{ExactClock, TimeService};

fn setup() -> (SQVM, TimeService) {
	let mut vm = SQVM::new(4096);
	let time = TimeService::new();
	time.install(&mut vm);
	(vm, time)
}

#[test]
fn time_unit_values() {
	let (vm, _t) = setup();
	vm.run_script(
		r#"
		assert(TimeUnit.Real == 0);
		assert(TimeUnit.Virtual == 1);
		assert(TimeUnit.Rounds == 2);
		"#,
		"time_unit",
	)
	.expect("TimeUnit values");
}

#[test]
fn clocks_start_at_zero_and_advance_with_tick() {
	let (vm, time) = setup();
	vm.run_script(
		r#"
		assert(Time.getRealTime() == 0);
		assert(Time.getVirtualTime() == 0);
		assert(Time.getRound() == 0);
		assert(Time.getFrame() == 0);
		assert(Time.getVirtualSpeed() == 1.0);
		"#,
		"boot",
	)
	.expect("clocks start at zero");

	time.tick(&vm, 16.0); // one ~60fps frame
	vm.run_script(
		r#"
		// Integer clocks are ms; the F variants, exact time and deltas are seconds.
		assert(Time.getRealTime() == 16);
		assert(Time.getRealTimeF() == 0.016);
		assert(Time.getExactTime() == 0.016);
		assert(Time.getVirtualTime() == 16);   // speed 1.0
		assert(Time.getVirtualTimeF() == 0.016);
		assert(Time.getDelta() == 0.016);
		assert(Time.getFrame() == 1);
		assert(Time.getFPS() == 1000.0 / 16.0);
		"#,
		"after_tick",
	)
	.expect("clocks advance with tick");
}

#[test]
fn a_live_exact_clock_moves_within_a_frame() {
	// As the original's: getExactTime keeps moving while a script runs, and the frame
	// clocks don't (the AI budgets its per-frame work with it).
	let (vm, time) = setup();
	time.set_exact_clock(ExactClock::Live);
	vm.run_script(
		r#"
		local e0 = Time.getExactTime(), r0 = Time.getRealTimeF();
		local start = e0, n = 0;
		while (Time.getExactTime() - start < 0.002) n++;   // spins ~2 ms of real time
		assert(Time.getExactTime() > e0);
		assert(Time.getRealTimeF() == r0);
		"#,
		"live",
	)
	.expect("a live exact clock moves within a frame");

	// Frame time again: still within the frame, it stands still.
	time.set_exact_clock(ExactClock::Frame);
	vm.run_script(
		r#"
		local e0 = Time.getExactTime();
		for (local i = 0; i < 100000; i++) {}
		assert(Time.getExactTime() == e0);
		"#,
		"frame",
	)
	.expect("the frame clock stands still within a frame");
}

#[test]
fn virtual_speed_scales_and_pauses_virtual_time() {
	let (vm, time) = setup();

	vm.run_script("Time.setVirtualSpeed(2.0);", "fast")
		.expect("set speed");
	time.tick(&vm, 10.0);
	vm.run_script(
		r#"
		assert(Time.getRealTime() == 10);
		assert(Time.getVirtualTime() == 20);   // 2x
		assert(Time.getVirtualDelta() == 0.02); // 10ms * 2.0, in seconds
		"#,
		"fast_check",
	)
	.expect("virtual scaled by speed");

	vm.run_script("Time.setVirtualSpeed(0.0);", "pause")
		.expect("pause");
	time.tick(&vm, 10.0);
	vm.run_script(
		r#"
		assert(Time.getRealTime() == 20);    // real keeps going
		assert(Time.getVirtualTime() == 20); // virtual frozen
		"#,
		"paused_check",
	)
	.expect("virtual paused at speed 0");

	// setVirtualTime takes seconds (what getVirtualTimeF gives), as the world does
	// around a battle: it parks its time, the battle runs from 0, then it's put back.
	vm.run_script("Time.setVirtualTime(0.5);", "settime")
		.expect("set time");
	vm.run_script("assert(Time.getVirtualTime() == 500); assert(Time.getVirtualTimeF() == 0.5);", "settime_check")
		.expect("virtual time set directly");
	vm.run_script("Time.setVirtualSpeed(1.0);", "speed").expect("speed");
	time.tick(&vm, 250.0);
	vm.run_script(
		r#"
		::parked <- Time.getVirtualTimeF();
		Time.setVirtualTime(0);
		assert(Time.getVirtualTime() == 0);
		Time.setVirtualTime(::parked);
		assert(Time.getVirtualTime() == 750);
		"#,
		"park",
	)
	.expect("parked and restored in seconds");
}

#[test]
fn schedule_event_real_fires_after_delay_with_userdata() {
	let (vm, time) = setup();
	vm.run_script(
		r#"
		::fired <- null;
		Time.scheduleEvent(TimeUnit.Real, 100, function (_ud) { ::fired = _ud; }, "payload");
		assert(Time.hasEventScheduled(TimeUnit.Real));
		assert(!Time.hasEventScheduled(TimeUnit.Virtual));
		"#,
		"schedule",
	)
	.expect("event scheduled");

	time.tick(&vm, 50.0); // not yet due
	vm.run_script("assert(::fired == null);", "early")
		.expect("not fired early");

	time.tick(&vm, 60.0); // now past 100ms total
	vm.run_script(
		r#"
		assert(::fired == "payload");                 // callback ran with its userdata
		assert(!Time.hasEventScheduled(TimeUnit.Real)); // and was consumed
		"#,
		"fired",
	)
	.expect("event fired once, with userdata");
}

#[test]
fn schedule_event_virtual_respects_speed() {
	let (vm, time) = setup();
	vm.run_script(
		r#"
		::v <- false;
		Time.setVirtualSpeed(2.0);
		Time.scheduleEvent(TimeUnit.Virtual, 100, function (_u) { ::v = true; }, null);
		"#,
		"sched_v",
	)
	.expect("scheduled virtual");
	// 40 real ms * 2.0 = 80 virtual ms — not yet 100
	time.tick(&vm, 40.0);
	vm.run_script("assert(::v == false);", "v_early")
		.expect("virtual not yet due");
	// +20 real ms * 2.0 = 40 more virtual -> 120 >= 100
	time.tick(&vm, 20.0);
	vm.run_script("assert(::v == true);", "v_fired")
		.expect("virtual event fired");
}

#[test]
fn schedule_event_rounds_fires_in_the_next_pass() {
	// `setRound` only sets the round (0x51d290); a due Rounds event fires in the next
	// frame's pass, as Real and Virtual ones do (0x5443e0).
	let (vm, time) = setup();
	vm.run_script(
		r#"
		::round_fired <- false;
		Time.scheduleEvent(TimeUnit.Rounds, 2, function (_u) { ::round_fired = true; }, null);
		Time.setRound(1);
		"#,
		"sched_r",
	)
	.expect("scheduled rounds");
	time.tick(&vm, 16.0);
	vm.run_script("assert(::round_fired == false);", "r1_check").expect("not due at round 1");
	vm.run_script("Time.setRound(2); assert(::round_fired == false);", "r2").expect("setRound doesn't fire");
	time.tick(&vm, 16.0);
	vm.run_script("assert(::round_fired == true); assert(Time.getRound() == 2);", "r2_check")
		.expect("fired in the pass after round 2");
}

#[test]
fn clear_events_removes_pending() {
	let (vm, time) = setup();
	vm.run_script(
		r#"
		Time.scheduleEvent(TimeUnit.Real, 10, function (_u) { ::x <- 1; }, null);
		assert(Time.hasEventScheduled(TimeUnit.Real));
		Time.clearEvents();
		assert(!Time.hasEventScheduled(TimeUnit.Real));
		"#,
		"clear",
	)
	.expect("events cleared");
	time.tick(&vm, 1000.0); // would have fired had it not been cleared
	vm.run_script(r#"assert(!("x" in getroottable()));"#, "not_fired")
		.expect("cleared event never fired");
}

#[test]
fn rust_side_clear_drops_pending_events() {
	let (vm, time) = setup();
	vm.run_script(
		r#"
		::z <- false;
		Time.scheduleEvent(TimeUnit.Real, 10, function (_u) { ::z = true; }, null);
		"#,
		"sched",
	)
	.expect("scheduled");
	time.clear(); // host-side teardown drops the pending event + its strong refs
	time.tick(&vm, 1000.0);
	vm.run_script("assert(::z == false);", "check")
		.expect("cleared event did not fire");
}

#[test]
fn bound_callback_keeps_its_environment() {
	let (vm, time) = setup();
	vm.run_script(
		r#"
		::obj <- { value = 7, result = null };
		::obj.report <- function (_ud) { this.result = this.value + _ud; };
		Time.scheduleEvent(TimeUnit.Real, 10, ::obj.report.bindenv(::obj), 5);
		"#,
		"bind",
	)
	.expect("scheduled bound callback");
	time.tick(&vm, 20.0);
	vm.run_script("assert(::obj.result == 12);", "bound_check")
		.expect("bound `this` (7) + userdata (5) == 12");
}

#[test]
fn has_event_scheduled_compares_the_internal_unit() {
	// The original stores Rounds events under internal unit 3 and compares the script's
	// value against it; 4 means any event (disassembled 0x51d2d0, probed).
	let (vm, _time) = setup();
	vm.run_script(
		r#"
		assert(!Time.hasEventScheduled(4));
		Time.scheduleEvent(TimeUnit.Rounds, 1, function (_u) {}, null);
		assert(!Time.hasEventScheduled(TimeUnit.Rounds));
		assert(Time.hasEventScheduled(3));
		assert(Time.hasEventScheduled(4));
		assert(!Time.hasEventScheduled(TimeUnit.Real));
		Time.scheduleEvent(TimeUnit.Virtual, 1, function (_u) {}, null);
		assert(Time.hasEventScheduled(TimeUnit.Virtual));
		"#,
		"has",
	)
	.expect("hasEventScheduled units");
}

#[test]
fn unknown_unit_schedules_nothing() {
	let (vm, time) = setup();
	vm.run_script(
		r#"
		::u <- false;
		Time.scheduleEvent(3, 0, function (_u) { ::u = true; }, null);
		Time.scheduleEvent(7, 0, function (_u) { ::u = true; }, null);
		assert(!Time.hasEventScheduled(4));
		"#,
		"bad_units",
	)
	.expect("unknown units");
	assert_eq!(time.pending_events(), 0);
	vm.run_script("Time.setRound(100);", "round").unwrap();
	time.tick(&vm, 1000.0);
	vm.run_script("assert(::u == false);", "check").expect("never fired");
}

#[test]
fn a_pass_fires_in_scheduling_order() {
	// One walk over the pending events in scheduling order, not deadline order.
	let (vm, time) = setup();
	vm.run_script(
		r#"
		::order <- [];
		Time.scheduleEvent(TimeUnit.Real, 300, function (_u) { ::order.push("a300"); }, null);
		Time.scheduleEvent(TimeUnit.Real, 100, function (_u) { ::order.push("b100"); }, null);
		Time.scheduleEvent(TimeUnit.Rounds, 1, function (_u) { ::order.push("c_round"); }, null);
		Time.setRound(1);
		"#,
		"sched",
	)
	.unwrap();
	time.tick(&vm, 500.0);
	vm.run_script(r#"assert(::order.len() == 3 && ::order[0] == "a300" && ::order[1] == "b100" && ::order[2] == "c_round");"#, "check")
		.expect("scheduling order");
}

#[test]
fn an_event_scheduled_by_a_callback_waits_for_the_next_pass() {
	let (vm, time) = setup();
	vm.run_script(
		r#"
		::child <- false;
		Time.scheduleEvent(TimeUnit.Real, 0, function (_u) {
			Time.scheduleEvent(TimeUnit.Real, 0, function (_u) { ::child = true; }, null);
			// Not counted until the pass ends (the original keeps it on a side list).
			::seen <- Time.hasEventScheduled(TimeUnit.Real);
		}, null);
		"#,
		"sched",
	)
	.unwrap();
	time.tick(&vm, 16.0);
	vm.run_script("assert(::child == false); assert(::seen == false);", "same_pass").expect("not in the same pass");
	assert_eq!(time.pending_events(), 1);
	time.tick(&vm, 16.0);
	vm.run_script("assert(::child == true);", "next_pass").expect("fired in the next pass");
}

#[test]
fn clear_events_in_a_callback_drops_the_rest_of_the_pass() {
	// What loading a campaign from a scheduled callback does (onBeforeDeserialize calls
	// Time.clearEvents()). The original frees the events under its own pass there
	// (undefined behaviour, it has crashed); we drop them, due or not.
	let (vm, time) = setup();
	vm.run_script(
		r#"
		::fired <- [];
		Time.scheduleEvent(TimeUnit.Real, 0, function (_u) {
			::fired.push("first");
			Time.scheduleEvent(TimeUnit.Real, 0, function (_u) { ::fired.push("scheduled_before_clear"); }, null);
			Time.clearEvents();
			Time.scheduleEvent(TimeUnit.Real, 0, function (_u) { ::fired.push("scheduled_after_clear"); }, null);
		}, null);
		Time.scheduleEvent(TimeUnit.Real, 0, function (_u) { ::fired.push("second"); }, null);
		Time.scheduleEvent(TimeUnit.Virtual, 0, function (_u) { ::fired.push("virtual"); }, null);
		"#,
		"sched",
	)
	.unwrap();
	time.tick(&vm, 16.0);
	vm.run_script(r#"assert(::fired.len() == 1 && ::fired[0] == "first");"#, "pass").expect("rest of the pass dropped");
	time.tick(&vm, 16.0);
	vm.run_script(r#"assert(::fired.len() == 2 && ::fired[1] == "scheduled_after_clear");"#, "next").expect("only the post-clear event survives");
	assert_eq!(time.pending_events(), 0);
}

#[test]
fn clear_events_drops_every_unit_and_nothing_else_does() {
	// Real time runs on, Virtual events freeze at speed 0 and fire when it resumes;
	// only clearEvents drops anything.
	let (vm, time) = setup();
	vm.run_script(
		r#"
		::fired <- [];
		Time.setVirtualSpeed(0.0);
		Time.scheduleEvent(TimeUnit.Virtual, 100, function (_u) { ::fired.push("virtual"); }, null);
		Time.scheduleEvent(TimeUnit.Real, 100, function (_u) { ::fired.push("real"); }, null);
		"#,
		"sched",
	)
	.unwrap();
	time.tick(&vm, 1000.0);
	vm.run_script(r#"assert(::fired.len() == 1 && ::fired[0] == "real");"#, "paused").expect("virtual frozen");
	vm.run_script("Time.setVirtualSpeed(1.0);", "resume").unwrap();
	time.tick(&vm, 100.0);
	vm.run_script(r#"assert(::fired.len() == 2 && ::fired[1] == "virtual");"#, "resumed").expect("virtual resumes");
	vm.run_script(
		r#"
		Time.scheduleEvent(TimeUnit.Real, 10, function (_u) { ::fired.push("x"); }, null);
		Time.scheduleEvent(TimeUnit.Virtual, 10, function (_u) { ::fired.push("x"); }, null);
		Time.scheduleEvent(TimeUnit.Rounds, 1, function (_u) { ::fired.push("x"); }, null);
		Time.clearEvents();
		assert(!Time.hasEventScheduled(4));
		Time.setRound(5);
		"#,
		"clear",
	)
	.unwrap();
	time.tick(&vm, 1000.0);
	vm.run_script("assert(::fired.len() == 2);", "cleared").expect("all units dropped");
}
