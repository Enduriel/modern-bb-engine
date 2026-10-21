//! End-to-end: a new campaign boots headless and is **seed-exact** against the original
//! game. Boots the menu, runs the genuine `startNewCampaign` for a fixed seed, and checks
//! the result against values captured from the original (2026-10-13):
//!
//! * the 2-arg `Math.rand` stream after the campaign's last string reseed (draw count
//!   and a hash of every draw's bounds and result), which covers assets, factions, the
//!   starting scenario and the faction simulation;
//! * every settlement's name, tile and type, in order;
//! * the terrain of the whole map, and the starting stash.
//!
//! Skips if no Steam install. Slow in debug builds (~20s): it runs all of worldgen.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use bb_engine::harness;

const SEED: &str = "rust-engine-test";

/// Counts 2-arg draws and hashes them, resetting on `seedRandomString`. All arithmetic
/// stays below 2^31, as the original's integers are 32-bit.
const RNG_TRACE: &str = r#"
::__rng <- { n = 0, h = 0 };
local origRand = ::Math.rand;
local origSeedS = ::Math.seedRandomString;
::Math.rand = function( ... )
{
	if (vargv.len() == 0) return origRand();
	local a = vargv[0], b = vargv[1];
	local r = origRand(a, b);
	local t = ::__rng;
	t.n++;
	t.h = (t.h * 33 + (r.tointeger() % 65521) + (a.tointeger() % 65521) * 3 + (b.tointeger() % 65521) * 5) % 1000003;
	if (t.h < 0) t.h += 1000003;
	return r;
};
::Math.seedRandomString = function( _s )
{
	::__rng.n = 0;
	::__rng.h = 0;
	return origSeedS(_s);
};
"#;

/// Reports the results as `OUT ...` lines.
const REPORT: &str = r#"
::print("OUT rng n=" + ::__rng.n + " h=" + ::__rng.h + "\n");
foreach (s in ::World.EntityManager.getSettlements())
{
	local tile = s.getTile();
	::print("OUT settlement " + s.getName() + " @" + tile.SquareCoords.X + "," + tile.SquareCoords.Y + " " + s.ClassName + "\n");
}
local sz = ::World.getMapSize();
local h = 0;
for (local x = 0; x < sz.X; x++)
	for (local y = 0; y < sz.Y; y++)
		h = (h * 7 + ::World.getTileSquare(x, y).Type + 1) % 1000003;
::print("OUT map " + sz.X + "x" + sz.Y + " typehash=" + h + "\n");
foreach (it in ::World.Assets.getStash().getItems())
	if (it != null) ::print("OUT stash " + it.getID() + " magic=" + it.m.MagicNumber + "\n");
"#;

const SETTLEMENTS: [&str; 21] = [
	"Seedeich @132,51 small_fishing_village",
	"Blankhoom @121,44 medium_fishing_village",
	"Eichenberg @63,64 small_mining_village",
	"Heuweiler @107,103 small_farming_village",
	"Eisengrab @67,106 small_mining_village",
	"Bitterfeld @97,87 small_farming_village",
	"Jungholz @113,78 small_lumber_village",
	"Friedewald @64,138 small_lumber_village",
	"Eidsvik @112,148 medium_tundra_village",
	"Erlach @86,53 medium_steppe_village",
	"Lydersted @76,139 large_tundra_village",
	"Sommersted @90,129 large_tundra_village",
	"Skallewold @133,144 small_tundra_fort",
	"Steinwacht @123,127 small_mountains_fort",
	"Turmburg @69,79 medium_mountains_fort",
	"Eidsvikholm @132,159 medium_snow_fort",
	"Wurmfeste @98,45 large_mountains_fort",
	"Lydersborg @110,135 large_tundra_fort",
	"Hakim Al Ramal @59,24 city_state",
	"Tarwa @120,20 city_state",
	"Dahab @84,25 city_state",
];

const STASH: [&str; 4] = [
	"supplies.ground_grains magic=33",
	"supplies.ground_grains magic=67",
	"accessory.bandage magic=95",
	"armor.head.fangshire magic=81",
];

#[test]
fn new_campaign_is_seed_exact_with_the_original() {
	let dir = Path::new(harness::STEAM_DATA_DIR);
	if !dir.exists() {
		bb_engine::harness::skip(format!("{} not present", dir.display()));
		return;
	}

	let mut engine = harness::open_install(dir).expect("open install");
	let out: Rc<RefCell<String>> = Rc::default();
	let sink = out.clone();
	engine.vm_mut().set_print_handler(move |s| sink.borrow_mut().push_str(s));

	harness::boot_menu(&engine).expect("boot the menu");
	harness::prepare_new_campaign(&engine, SEED).expect("prepare the campaign");
	engine.run_script(RNG_TRACE, "rng-trace").expect("install the RNG trace");
	harness::start_new_campaign(&engine).expect("startNewCampaign");
	engine.run_script(REPORT, "report").expect("report");

	let text = out.borrow();
	let lines: Vec<&str> = text.lines().filter_map(|l| l.strip_prefix("OUT ")).collect();
	let with = |prefix: &str| -> Vec<String> {
		lines
			.iter()
			.filter_map(|l| l.strip_prefix(prefix))
			.map(str::to_string)
			.collect()
	};

	assert_eq!(with("rng "), ["n=5442 h=72898"], "post-reseed RNG stream");
	assert_eq!(with("settlement "), SETTLEMENTS);
	assert_eq!(with("map "), ["140x170 typehash=719994"]);
	assert_eq!(with("stash "), STASH);
}
