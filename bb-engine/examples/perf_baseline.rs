//! The performance baseline, our engine's side: the same measurements as `parity/perf/perf.py` takes
//! in the original, printed next to the original's (`parity/perf/original.txt`).
//!
//!   cargo run --release -p bb-engine --example perf_baseline [-- <data dir>]
//!
//! - menu: process start to the main menu; resident memory there; the script benches
//!   (`parity/perf/benches.nut`), each run five times, minimum and median.
//! - campaign (seed `rust-engine-test`, started from the menu as the probe harness
//!   does): start to `world_state.init()` done; memory; the world benches
//!   (`benches_world.nut`); save and load times.
//!
//! Not comparable one to one: the original's memory includes Wine, the renderer and the
//! UI (ours is headless), its saves go to disk (ours stay in memory), and it has no
//! counterpart here for the frame rate. Benches are timed from Rust; the original
//! times them with its live clock.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use bb_engine::harness::{self, GameEngine};
use squirrel::SqTable;

const SEED: &str = "rust-engine-test";
const FRAME_MS: f64 = 16.0;

/// How the probe harness's campaign mode starts the campaign.
const START_CAMPAIGN: &str = r#"::RootState.get("MainMenuState").campaign_menu_module_onStartPressed({
	Name = "Probe Company",
	Banner = ::Const.PlayerBanners[0],
	Difficulty = 1,
	EconomicDifficulty = 1,
	BudgetDifficulty = 1,
	Ironman = false,
	ExplorationMode = false,
	GreaterEvil = ::Const.World.GreaterEvilType.Random,
	PermanentDestruction = false,
	Seed = "@SEED@",
	StartingScenario = "scenario.early_access"
});"#;

fn perf_dir() -> PathBuf {
	Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parity/perf")
}

/// `VmRSS` or `VmHWM` of this process, in MB.
fn memory_mb(field: &str) -> f64 {
	let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
	status
		.lines()
		.find_map(|l| l.strip_prefix(field).and_then(|r| r.strip_prefix(':')))
		.and_then(|r| r.split_whitespace().next()?.parse::<f64>().ok())
		.map_or(0.0, |kb| kb / 1024.0)
}

fn ms(since: Instant) -> f64 {
	since.elapsed().as_secs_f64() * 1000.0
}

/// Load a bench file as game code (single-precision floats, as the original runs it).
fn load(engine: &GameEngine, file: &str) {
	let source = std::fs::read_to_string(perf_dir().join(file)).unwrap();
	let vm = engine.vm();
	let closure = vm.compile(&source, file).unwrap_or_else(|e| panic!("{file}: {e}"));
	closure.set_f32_math(vm, true);
	closure.call(vm, ()).unwrap_or_else(|e| panic!("{file}: {e}"));
}

/// Run every bench of `list` five times, as `original_common.nut` does.
fn run_benches(engine: &GameEngine, list: &str, prefix: &str, out: &mut BTreeMap<String, String>) {
	engine.run_script(&format!("::__perfCount <- ::{list}.len();"), "perf").unwrap();
	let count = SqTable::root(engine.vm()).get::<i64>("__perfCount").unwrap();
	for i in 0..count {
		engine.run_script(&format!("::__perfName <- ::{list}[{i}].Name;"), "perf").unwrap();
		let name = SqTable::root(engine.vm()).get::<String>("__perfName").unwrap();
		let call = format!("::{list}[{i}].Run();");
		let mut times: Vec<f64> = (0..5)
			.map(|_| {
				let t = Instant::now();
				engine.run_script(&call, "perf").unwrap_or_else(|e| panic!("{name}: {e}"));
				ms(t)
			})
			.collect();
		times.sort_by(f64::total_cmp);
		out.insert(format!("{prefix} bench {name} min_ms"), format!("{:.1}", times[0]));
		out.insert(format!("{prefix} bench {name} median_ms"), format!("{:.1}", times[2]));
	}
}

fn original() -> BTreeMap<String, String> {
	let text = std::fs::read_to_string(perf_dir().join("original.txt")).unwrap_or_default();
	text.lines()
		.filter(|l| !l.starts_with('#'))
		.filter_map(|l| l.split_once(" = "))
		.map(|(k, v)| (k.to_string(), v.to_string()))
		.collect()
}

fn main() {
	let start = Instant::now();
	let data = std::env::args().nth(1).unwrap_or_else(|| harness::STEAM_DATA_DIR.to_string());
	let mut ours = BTreeMap::new();

	let engine = harness::open_install(Path::new(&data)).expect("open install");
	harness::boot_menu(&engine).expect("boot the menu");
	ours.insert("menu launch_to_menu_s".into(), format!("{:.2}", start.elapsed().as_secs_f64()));
	ours.insert("menu rss_mb at menu".into(), format!("{:.0}", memory_mb("VmRSS")));
	load(&engine, "benches.nut");
	run_benches(&engine, "PerfBenches", "menu", &mut ours);
	ours.insert("menu rss_mb peak".into(), format!("{:.0}", memory_mb("VmHWM")));

	// The campaign, from the menu's start button to world_state.init() done.
	let t = Instant::now();
	engine
		.run_script(&START_CAMPAIGN.replace("@SEED@", SEED), "start")
		.expect("start the campaign");
	loop {
		engine.init_pending_states().expect("init the new states");
		if engine.root_state().state("WorldState").is_some() {
			break;
		}
		assert!(t.elapsed().as_secs() < 120, "the campaign's world never started");
		harness::run_frame(&engine, FRAME_MS).expect("a frame");
	}
	ours.insert("campaign new_campaign_s".into(), format!("{:.2}", t.elapsed().as_secs_f64()));
	ours.insert("campaign rss_mb at world".into(), format!("{:.0}", memory_mb("VmRSS")));
	load(&engine, "benches_world.nut");
	run_benches(&engine, "PerfWorldBenches", "campaign", &mut ours);

	let t = Instant::now();
	engine
		.run_script("::World.State.saveCampaign(\"bb_probe_perf\");", "save")
		.expect("save");
	ours.insert("campaign save_ms".into(), format!("{:.1}", ms(t)));
	// loadCampaign ignores a load within 4 s of the last one by real time; ours has only
	// run a few seconds of frames.
	let t = Instant::now();
	engine
		.run_script(
			"::World.State.m.CampaignLoadTime = -100.0; ::World.State.loadCampaign(\"bb_probe_perf\");",
			"load",
		)
		.expect("load");
	ours.insert("campaign load_ms".into(), format!("{:.1}", ms(t)));
	ours.insert("campaign rss_mb peak".into(), format!("{:.0}", memory_mb("VmHWM")));

	let original = original();
	println!("{:<44} {:>10} {:>10} {:>8}", "measure", "ours", "original", "ratio");
	let keys: std::collections::BTreeSet<&String> = ours.keys().chain(original.keys()).collect();
	for key in keys {
		let a = ours.get(key).map_or("-", String::as_str);
		let b = original.get(key).map_or("-", String::as_str);
		let ratio = match (a.parse::<f64>(), b.parse::<f64>()) {
			(Ok(a), Ok(b)) if a > 0.0 && b > 0.0 => format!("{:.2}x", b / a),
			_ => String::new(),
		};
		println!("{key:<44} {a:>10} {b:>10} {ratio:>8}");
	}
	println!("(ratio = original / ours: above 1 means ours is faster or smaller)");
}
