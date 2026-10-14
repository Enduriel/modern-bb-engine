//! Run a campaign-mode probe (written for the original's probe runner) in this engine,
//! to diff against the original's run: a new campaign started as the probe harness starts one (scenario
//! `early_access`, difficulties 1, `GreaterEvil` random, map seed `seed`), then the probe
//! script at the same point (after the world state's init). The probe's `PROBE ...`
//! lines print as the original's runner prints them.
//!
//! Run: `cargo run --release -p bb-engine --example campaign_probe -- <probe.nut> <seed>`

use std::path::Path;

use bb_engine::harness;

fn main() {
	let mut args = std::env::args().skip(1);
	let probe = args.next().expect("usage: campaign_probe <probe.nut> <seed>");
	let seed = args.next().expect("usage: campaign_probe <probe.nut> <seed>");
	let code = std::fs::read_to_string(&probe).unwrap_or_else(|e| panic!("{probe}: {e}"));
	let dir = Path::new(harness::STEAM_DATA_DIR);
	let engine = harness::open_install(dir).expect("open install");
	harness::boot_menu(&engine).expect("boot the menu");
	harness::prepare_new_campaign(&engine, &seed).expect("prepare the campaign");
	harness::start_new_campaign(&engine).expect("start the campaign");
	if let Err(e) = engine.run_script(&code, "probe") {
		println!("PROBE_FAIL {e}");
	}
	println!("PROBE_DONE");
}
