//! End-to-end: a new campaign runs, is saved and loaded back, and keeps
//! running. The world after the load must read the same as before it (clock, assets,
//! every entity's id and position, the company's brothers with their skills and items,
//! contracts, factions, the fog, the fog-of-war mask and the region names), and
//! another game minute must pass without a script error.
//!
//! Byte-identical re-saves aren't expected: loading re-runs script code that reorders
//! a few things (heapsort ties in skill lists, flag tables' iteration order), as the
//! original's scripts would.
//!
//! Skips if no Steam install. Slow in debug builds: it runs all of worldgen first.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use bb_engine::harness;

/// One line describing the world, printed as `OUT ...`.
const SUMMARY: &str = r#"
local t = ::World.getTime();
local s = "time=" + t.Time + " money=" + ::World.Assets.getMoney() + " food=" + ::World.Assets.getFood();
local ents = ::World.getAllEntitiesAtPos(::World.State.getPlayer().getPos(), 100000.0);
s += " entities=" + ents.len();
foreach (e in ents) s += " " + e.getID() + "@" + e.getPos().X + "," + e.getPos().Y;
foreach (b in ::World.getPlayerRoster().getAll())
	s += " bro:" + b.getName() + "/" + b.getLevel() + "/" + b.getHitpoints() + "/" + b.getSkills().m.Skills.len() + "/" + b.getItems().getAllItems().len();
s += " contracts=" + ::World.Contracts.getOpenContracts().len() + " factions=" + ::World.FactionManager.getFactions().len();
local discovered = 0;
local size = ::World.getMapSize();
for (local x = 0; x < size.X; x++)
	for (local y = 0; y < size.Y; y++)
		if (::World.getTileSquare(x, y).IsDiscovered) discovered++;
s += " discovered=" + discovered;
::print("OUT " + s + "\n");
"#;

fn summary(engine: &harness::GameEngine, out: &Rc<RefCell<String>>) -> String {
	out.borrow_mut().clear();
	engine.run_script(SUMMARY, "summary").expect("summary");
	let text = out.borrow();
	text.lines()
		.find_map(|l| l.strip_prefix("OUT "))
		.expect("summary line")
		.to_string()
}

fn run(engine: &harness::GameEngine, frames: u32) {
	for frame in 0..frames {
		harness::run_frame(engine, 1000.0 / 60.0).unwrap_or_else(|e| panic!("frame {frame}: {e}"));
	}
}

#[test]
fn a_campaign_survives_save_and_load() {
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
	harness::unpause_world(&engine).expect("unpause");
	run(&engine, 3600);

	let regions = engine.world_scene().region_texts;
	assert!(!regions.is_empty());
	engine.run_script("::World.State.saveCampaign(\"bb_probe_milestone\");", "save").expect("save");
	let before = summary(&engine, &out);
	engine.run_script("::World.State.loadCampaign(\"bb_probe_milestone\");", "load").expect("load");
	let after = summary(&engine, &out);
	assert_eq!(before, after, "the loaded world differs");
	assert_eq!(engine.world_scene().region_texts, regions, "region names survive");
	// The fog mask comes back with the map: it uploads whole, uncovered around the
	// company and fogged elsewhere.
	let mask = engine.world_grid().take_fog_patch().expect("the loaded mask");
	assert_eq!((mask.width, mask.height), (2048, 2048));
	assert!(mask.alpha.contains(&0) && mask.alpha.contains(&255), "the fog mask survives");

	// The loaded world keeps running (loading pauses it, like the original).
	harness::unpause_world(&engine).expect("unpause after load");
	run(&engine, 3600);
	let later = summary(&engine, &out);
	assert_ne!(after, later, "nothing happened after the load");
}
