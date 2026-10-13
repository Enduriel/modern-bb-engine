//! Tests for the `Time` service: the three clocks, `scheduleEvent` firing, and
//! the host-driven `tick`/`setRound` advancement.

use squirrel::SQVM;
use squirrel_bb::TimeService;

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

	vm.run_script("Time.setVirtualTime(500);", "settime")
		.expect("set time");
	vm.run_script("assert(Time.getVirtualTime() == 500);", "settime_check")
		.expect("virtual time set directly");
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
fn schedule_event_rounds_fires_on_set_round() {
	let (vm, time) = setup();
	let _ = time; // rounds advance via Time.setRound, not tick
	vm.run_script(
		r#"
		::round_fired <- false;
		Time.scheduleEvent(TimeUnit.Rounds, 2, function (_u) { ::round_fired = true; }, null);
		"#,
		"sched_r",
	)
	.expect("scheduled rounds");
	vm.run_script("Time.setRound(1);", "r1").expect("round 1");
	vm.run_script("assert(::round_fired == false);", "r1_check")
		.expect("not due at round 1");
	vm.run_script("Time.setRound(2);", "r2").expect("round 2");
	vm.run_script("assert(::round_fired == true);", "r2_check")
		.expect("fired at round 2");
	vm.run_script("assert(Time.getRound() == 2);", "round_now")
		.expect("round updated");
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
