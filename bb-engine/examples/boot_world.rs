//! Drive the REAL new-game flow into a running (paused) campaign world, headless:
//! boot the menu, add `WorldState` (its `onInit` stands up the 11 world managers), and
//! run the genuine `startNewCampaign` (worldgen, `Assets.init`, faction creation, the
//! starting scenario, the faction simulation). The world UI is stubbed (logic-first);
//! see [`bb_engine::harness`]. Reports how far the boot gets: an error names the next
//! missing native.
//!
//! Run: `cargo run -p bb-engine --example boot_world -- [data-dir] [--seed S]
//! [--setup FILE] [--after FILE] [--frames N] [--end FILE]`. With `BB_SAVE_DIR` set,
//! saves go to that directory instead of memory.

use std::path::PathBuf;

use bb_engine::harness;

/// `--setup` runs just before `startNewCampaign` (e.g. to instrument worldgen), `--after`
/// once it returns: the same hooks the probe runner offers in the original, so
/// traces compare 1:1.
struct Args {
	dir: PathBuf,
	seed: String,
	setup: Option<String>,
	after: Option<String>,
	/// Frames of the running world to simulate after the boot (60 fps steps).
	frames: u32,
	/// Runs after the frames.
	end: Option<String>,
}

fn parse_args() -> Args {
	let mut args = Args {
		dir: PathBuf::from(harness::STEAM_DATA_DIR),
		seed: "rust-engine-test".to_string(),
		setup: None,
		after: None,
		frames: 0,
		end: None,
	};
	let mut it = std::env::args().skip(1);
	while let Some(a) = it.next() {
		let read = |f: Option<String>| std::fs::read_to_string(f.expect("missing value")).expect("read script");
		match a.as_str() {
			"--seed" => args.seed = it.next().expect("--seed value"),
			"--setup" => args.setup = Some(read(it.next())),
			"--after" => args.after = Some(read(it.next())),
			"--end" => args.end = Some(read(it.next())),
			"--frames" => args.frames = it.next().and_then(|n| n.parse().ok()).expect("--frames N"),
			dir => args.dir = PathBuf::from(dir),
		}
	}
	args
}

fn main() {
	let args = parse_args();
	let mut engine = harness::open_install(&args.dir).expect("open install");
	if let Ok(dir) = std::env::var("BB_SAVE_DIR") {
		engine.saves().set_directory(dir.into());
	}
	engine.vm_mut().set_print_handler(|s| eprint!("{s}"));
	engine.vm_mut().install_standard_error_handlers();

	harness::boot_menu(&engine).expect("boot menu");
	println!("menu booted; driving new-game world boot…");

	let result = harness::prepare_new_campaign(&engine, &args.seed)
		.and_then(|()| match &args.setup {
			Some(setup) => engine.run_script(setup, "setup"),
			None => Ok(()),
		})
		.and_then(|()| harness::start_new_campaign(&engine))
		.and_then(|()| match &args.after {
			Some(after) => engine.run_script(after, "after"),
			None => Ok(()),
		})
		.and_then(|()| {
			if args.frames > 0 {
				harness::unpause_world(&engine)?;
			}
			for n in 0..args.frames {
				harness::run_frame(&engine, 1000.0 / 60.0).map_err(|e| format!("frame {n}: {e}"))?;
			}
			Ok(())
		})
		.and_then(|()| match &args.end {
			Some(end) => engine.run_script(end, "end"),
			None => Ok(()),
		});
	match result {
		Ok(()) => {
			println!("✅ new-game world boot ran to completion");
			let (w, h) = engine.world_grid().dims();
			println!("   grid {w}x{h}; entities spawned = {}", engine.world_entities().count());
			let _ = engine.run_script(
				r#"
				this.logInfo("settlements = " + this.World.EntityManager.getSettlements().len());
				this.logInfo("player = " + (this.World.State.m.Player != null ? "set" : "null"));
				local t = this.World.getTime();
				this.logInfo("world time: day " + t.Days + " " + t.Hours + ":" + t.Minutes + " (TimeOfDay " + t.TimeOfDay + ", speed " + this.World.getSpeedMult() + ")");
				local all = this.World.getAllEntitiesAtPos(this.World.State.getPlayer().getPos(), 100000.0);
				local parties = 0;
				foreach (e in all) if (e.isParty()) parties++;
				this.logInfo("live entities = " + all.len() + ", parties = " + parties);
				local discovered = 0;
				foreach (e in all) if (e.isDiscovered()) discovered++;
				local pl = this.World.State.getPlayer();
				this.logInfo("player pos " + pl.getPos().X + "," + pl.getPos().Y + " vision " + pl.getVisionRadius() + " hasPath " + (pl.getPath() != null) + " dest " + (pl.m.Destination != null));
				this.logInfo("discovered = " + discovered + ", LocationsDiscovered stat = " + this.World.Statistics.getFlags().get("LocationsDiscovered"));
				"#,
				"diag",
			);
		}
		Err(e) => println!("⏹  world boot stopped at the next gap:\n   {e}"),
	}
}
