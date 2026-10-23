//! The script VM's own work, timed in the four places it shows: the boot to
//! the main menu, a new campaign (worldgen, seed `rust-engine-test`), the running world
//! (1,800 frames unpaused, the scripted player answering events) and a battle the AI
//! fights for both sides (the nearest hostile party, 1,800 frames). Every phase is the
//! same work on every run (virtual time is fixed per frame), so two builds compare by
//! their times.
//!
//!   cargo run --release -p bb-engine --example vm_workloads [-- [--mods] [<data dir>]]
//!
//! `--mods` mounts the install's mods too (`harness::mod_archives`) and times the boot
//! only (their hooks applied, the menu up): a modded campaign is started through the
//! menus, not this harness's shortcut.
//!
//! Each phase prints `PHASE <name> <ms> <start> <end> <cpu ms>`: the wall time, its start
//! and end in `CLOCK_MONOTONIC` nanoseconds (so a sampling profiler's samples can be split
//! by phase), and the thread's CPU time (less disturbed by other work on the machine).

use std::path::Path;
use std::time::Instant;

use bb_engine::harness;

const SEED: &str = "rust-engine-test";
const FRAMES: usize = 1800;

#[repr(C)]
struct Timespec {
	sec: i64,
	nsec: i64,
}

unsafe extern "C" {
	fn clock_gettime(clock: i32, ts: *mut Timespec) -> i32;
}

/// A clock in nanoseconds: `CLOCK_MONOTONIC` (1) or `CLOCK_THREAD_CPUTIME_ID` (3).
fn clock_ns(clock: i32) -> i64 {
	let mut ts = Timespec { sec: 0, nsec: 0 };
	// SAFETY: a valid out pointer; both clocks exist on Linux.
	unsafe { clock_gettime(clock, &mut ts) };
	ts.sec * 1_000_000_000 + ts.nsec
}

fn monotonic_ns() -> i64 {
	clock_ns(1)
}

fn phase<T>(name: &str, f: impl FnOnce() -> T) -> T {
	let (start, cpu, t) = (monotonic_ns(), clock_ns(3), Instant::now());
	let out = f();
	let cpu_ms = (clock_ns(3) - cpu) as f64 / 1e6;
	println!("PHASE {name} {:.1} {start} {} {cpu_ms:.1}", t.elapsed().as_secs_f64() * 1000.0, monotonic_ns());
	out
}

fn frames(engine: &harness::GameEngine) {
	for n in 0..FRAMES {
		harness::run_frame(engine, 1000.0 / 60.0).unwrap_or_else(|e| panic!("frame {n}: {e}"));
	}
}

fn main() {
	let args: Vec<String> = std::env::args().skip(1).collect();
	let mods = args.iter().any(|a| a == "--mods");
	let data = args.iter().find(|a| *a != "--mods").cloned().unwrap_or_else(|| harness::STEAM_DATA_DIR.to_string());
	let engine = phase("boot", || {
		let engine = if mods {
			harness::open_install_with_mods(Path::new(&data))
		} else {
			harness::open_install(Path::new(&data))
		}
		.expect("open install");
		harness::boot_menu(&engine).expect("boot the menu");
		engine
	});
	if mods {
		return;
	}
	phase("campaign", || {
		harness::prepare_new_campaign(&engine, SEED).expect("prepare");
		harness::start_new_campaign(&engine).expect("start");
	});
	harness::unpause_world(&engine).expect("unpause");
	phase("world", || frames(&engine));
	engine.run_script("::__autoplayFight = true;", "fight").expect("fight");
	harness::engage_nearest_hostile(&engine).expect("engage");
	phase("battle", || frames(&engine));
	engine
		.run_script(
			r#"::logInfo("battle round " + ::Time.getRound() + ", finished " + ::Tactical.Entities.isCombatFinished());"#,
			"status",
		)
		.expect("status");
}
