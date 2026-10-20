//! Loading the same campaign over and over doesn't grow memory (vanilla's engine leaks
//! here). The procedure is the one the original was measured with (a reload probe):
//! the original's save of a new campaign
//! (`harness::ORIGINAL_SAVES_DIR`, through the original-format import) is loaded 15 times
//! from inside the campaign, as the in-game menu's Load does, then 8 times by quitting to
//! the main menu and loading it from there. A save in this engine's own format is then
//! loaded 10 times as well.
//!
//! After a warm-up, each load leaves the heap where the last one did, the engine holds the
//! same entities, thinkers, scheduled events and UI connections, and no cycle of script
//! objects is left for the collector. The heap is glibc's count of bytes in use
//! (`mallinfo2`): exact, unlike the resident set, which is only a loose backstop here.
//! One test in its own binary, so no other test allocates meanwhile. Skips without the
//! Steam install or the original's saves.

use std::path::Path;

use bb_engine::{harness, tactical_player as player};

const MENU: &str = "::RootState.get(\"MainMenuState\").m.MainMenuScreen";
const AT_MENU: &str = "::RootState.get(\"WorldState\") == null && ::RootState.get(\"MainMenuState\").m.MainMenuScreen.isVisible() && !::RootState.get(\"MainMenuState\").m.MainMenuScreen.isAnimating()";
const LOADED: &str = "::RootState.get(\"WorldState\") != null && (\"State\" in ::World) && ::World.State != null && ::World.State.getPlayer() != null && !::LoadingScreen.isVisible()";

/// Loads before the measured ones: the first loads fill caches and pools for good.
const WARM_UP: usize = 3;
/// As the original was measured.
const LOADS_IN_CAMPAIGN: usize = 15;
const LOADS_FROM_MENU: usize = 8;
const LOADS_OWN_FORMAT: usize = 10;
/// Real time between loads: the world state ignores a load within 4 s of the last.
const FRAMES_BETWEEN: u32 = 360;

#[repr(C)]
struct Mallinfo2 {
	arena: usize,
	ordblks: usize,
	smblks: usize,
	hblks: usize,
	hblkhd: usize,
	usmblks: usize,
	fsmblks: usize,
	uordblks: usize,
	fordblks: usize,
	keepcost: usize,
}

unsafe extern "C" {
	fn mallinfo2() -> Mallinfo2;
}

/// Heap bytes in use: allocated chunks, and the large ones mapped on their own.
fn heap_bytes() -> usize {
	let m = unsafe { mallinfo2() };
	m.uordblks + m.hblkhd
}

fn rss_kb() -> usize {
	std::fs::read_to_string("/proc/self/status")
		.ok()
		.and_then(|s| s.lines().find(|l| l.starts_with("VmRSS:")).and_then(|l| l.split_whitespace().nth(1)?.parse().ok()))
		.unwrap_or(0)
}

fn frames(engine: &harness::GameEngine, n: u32) {
	for _ in 0..n {
		harness::run_frame(engine, 1000.0 / 60.0).unwrap();
	}
}

fn until(engine: &harness::GameEngine, max: u32, what: &str, cond: &str) {
	for _ in 0..max {
		if player::eval_i64(engine, &format!("({cond}) ? 1 : 0")).unwrap() == Some(1) {
			return;
		}
		frames(engine, 1);
	}
	panic!("{what}: still not true after {max} frames: {cond}");
}

/// What the engine holds.
#[derive(Debug, PartialEq)]
struct Held {
	entities: usize,
	thinkers: usize,
	units: usize,
	events: usize,
	connections: usize,
	states: Vec<String>,
}

fn held(engine: &harness::GameEngine) -> Held {
	Held {
		entities: engine.world_entities().count(),
		thinkers: engine.world_entities().thinker_count(),
		units: engine.tactical_entities().unit_count(),
		events: engine.time().pending_events(),
		connections: engine.ui().connection_count(),
		states: engine.root_state().state_names(),
	}
}

/// One load's footprint.
struct After {
	heap: usize,
	held: Held,
	/// Objects only reference cycles kept alive (freed by the collector).
	cycles: i64,
}

fn after_load(engine: &harness::GameEngine) -> After {
	let cycles = engine.vm().collect_garbage();
	After {
		heap: heap_bytes(),
		held: held(engine),
		cycles,
	}
}

/// Every load after the warm-up leaves memory and the engine where the first of them did.
fn assert_flat(kind: &str, loads: &[After]) {
	let mb = |b: usize| b as f64 / 1_048_576.0;
	for (i, a) in loads.iter().enumerate() {
		eprintln!("{kind} load {:2}: heap {:.2} MB, cycles freed {}, {:?}", i + 1, mb(a.heap), a.cycles, a.held);
	}
	let measured = &loads[WARM_UP..];
	let base = &measured[0];
	for (i, a) in measured.iter().enumerate() {
		assert_eq!(a.held, base.held, "{kind} load {}: the engine holds more", WARM_UP + i + 1);
		assert_eq!(a.cycles, 0, "{kind} load {}: script objects left in cycles", WARM_UP + i + 1);
	}
	let (lo, hi) = measured.iter().fold((usize::MAX, 0), |(lo, hi), a| (lo.min(a.heap), hi.max(a.heap)));
	let last = measured.last().unwrap().heap;
	eprintln!(
		"{kind}: heap {:.2} → {:.2} MB over {} loads (range {:.2} MB)",
		mb(base.heap),
		mb(last),
		measured.len(),
		mb(hi - lo)
	);
	// Allocator noise is a few KB a load; a leak of one campaign's objects is megabytes.
	assert!(
		last <= base.heap + 1_048_576,
		"{kind}: the heap grew {:.2} MB over {} loads",
		mb(last.saturating_sub(base.heap)),
		measured.len()
	);
}

/// Load save `name` from inside the campaign, `n` times, 6 s apart.
fn load_in_campaign(engine: &harness::GameEngine, name: &str, n: usize) -> Vec<After> {
	(0..n)
		.map(|_| {
			frames(engine, FRAMES_BETWEEN);
			engine.run_script(&format!("::World.State.loadCampaign(\"{name}\");"), "load").unwrap();
			frames(engine, 1);
			after_load(engine)
		})
		.collect()
}

/// Load save `name` from the main menu (quitting to it first if a campaign runs).
fn load_from_menu(engine: &harness::GameEngine, name: &str) {
	if player::eval_i64(engine, "::RootState.get(\"WorldState\") != null ? 1 : 0").unwrap() == Some(1) {
		engine.run_script("::World.State.exitGame();", "quit to the menu").unwrap();
	}
	until(engine, 600, "at the menu", AT_MENU);
	engine.run_script(&format!("{MENU}.getMainMenuModule().onLoadCampaignButtonPressed();"), "load menu").unwrap();
	frames(engine, 1);
	engine
		.run_script(&format!("{MENU}.getLoadCampaignMenuModule().onLoadButtonPressed(\"{name}\");"), "load")
		.unwrap();
	until(engine, 600, "the campaign loading", LOADED);
	frames(engine, 1);
}

#[test]
fn reloading_a_campaign_does_not_grow_memory() {
	let from = Path::new(harness::ORIGINAL_SAVES_DIR).join("new_campaign.sav");
	if !Path::new(harness::STEAM_DATA_DIR).exists() || !from.exists() {
		eprintln!("skipping: needs the Steam install and {}", from.display());
		return;
	}
	// The fixture, where the engine may write.
	let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("reload_memory");
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).unwrap();
	std::fs::copy(&from, dir.join("bb_probe_reload.sav")).unwrap();

	let mut engine = harness::open_install(Path::new(harness::STEAM_DATA_DIR)).expect("open install");
	engine.vm_mut().install_standard_error_handlers();
	engine.saves().set_directory(dir.clone());
	harness::boot_menu(&engine).expect("boot the menu");
	harness::install_autoplay(&engine).expect("autoplay");
	until(&engine, 600, "the main menu", AT_MENU);
	// The world state ignores a load within 4 s of the game's start.
	until(&engine, 600, "4 s at the menu", "::Time.getRealTimeF() > 4.5");
	load_from_menu(&engine, "bb_probe_reload");
	let rss_start = rss_kb();

	let loads = load_in_campaign(&engine, "bb_probe_reload", LOADS_IN_CAMPAIGN);
	assert_flat("in-campaign", &loads);

	let loads: Vec<After> = (0..LOADS_FROM_MENU)
		.map(|_| {
			load_from_menu(&engine, "bb_probe_reload");
			after_load(&engine)
		})
		.collect();
	assert_flat("from-menu", &loads);

	// A save in this engine's own format.
	engine.run_script("::World.State.saveCampaign(\"bb_probe_reload_ours\");", "save").unwrap();
	let loads = load_in_campaign(&engine, "bb_probe_reload_ours", LOADS_OWN_FORMAT);
	assert_flat("own-format", &loads);

	let rss_end = rss_kb();
	eprintln!("resident set: {} → {} MB", rss_start / 1024, rss_end / 1024);
	// Loose: the allocator keeps freed pages, and the menu round trips touch new ones.
	assert!(rss_end < rss_start + 100 * 1024, "the resident set grew {} MB", (rss_end - rss_start) / 1024);
	let _ = std::fs::remove_dir_all(&dir);
}
