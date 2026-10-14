//! End-to-end: the company is played by a scripted player whose commands go in the way
//! a player's do (`bb_engine::tactical_player`): clicks on tiles through the engine's
//! mouse input into the tactical state, keys through its key input, the turn bar's
//! skill, Wait and End Turn buttons, and the inventory's item moves. In a battle against
//! bandit raiders every brother waits once in the first round, then attacks the nearest
//! enemy it can reach with its first usable attack, or walks up to it, or ends its turn.
//! In another each brother first takes a fire bomb out of its bag and throws it. A
//! third battle retreats on the first turn.
//!
//! Skips if no Steam install. Slow in debug builds: each test runs all of worldgen first.

use std::path::Path;

use bb_engine::{harness, tactical_player as player};
use squirrel::SqTable;

/// Sets up a scripted battle against bandit raiders on the plains, with the company left
/// to the player (`::__autoplayPlayer` does nothing, so the autoplay doesn't end its
/// turns), and `::__decide()`, which picks the acting brother's next command.
const SETUP: &str = r#"
::__autoplayPlayer = function() {};
::__prefer <- [];
::__decide <- function()
{
	local e = ::Tactical.TurnSequenceBar.getActiveEntity();
	local enemies = [];
	foreach (f in ::Tactical.Entities.getAllInstances())
		foreach (a in f)
			if (a.isAlive() && a.isPlacedOnMap() && !a.isAlliedWith(e) && !a.isHiddenToPlayer()) enemies.push(a);
	local my = e.getTile();
	enemies.sort(function( a, b ) {
		local da = my.getDistanceTo(a.getTile()), db = my.getDistanceTo(b.getTile());
		return da < db ? -1 : (da > db ? 1 : 0);
	});
	local actives = [];
	foreach (id in ::__prefer) { local sk = e.getSkills().getSkillByID(id); if (sk != null) actives.push(sk); }
	foreach (sk in e.getSkills().queryActives()) if (::__prefer.find(sk.getID()) == null) actives.push(sk);
	foreach (sk in actives)
	{
		if (!sk.isAttack() || !sk.isUsable() || !sk.isAffordable()) continue;
		foreach (en in enemies)
			if (sk.isUsableOn(en.getTile()))
				return "skill " + sk.getID() + " " + en.getTile().SquareCoords.X + " " + en.getTile().SquareCoords.Y;
	}
	if (enemies.len() != 0 && e.getActionPoints() >= 4)
	{
		local target = enemies[0].getTile();
		for (local d = 0; d < 6; d++)
		{
			if (!target.hasNextTile(d)) continue;
			local t = target.getNextTile(d);
			if (t.IsEmpty) return "move " + t.SquareCoords.X + " " + t.SquareCoords.Y;
		}
	}
	return "end";
};
::__offerBattle <- function()
{
	::World.State.m.IsAutosaving = false;
	local st = ::World.State.get();
	local p = st.getLocalCombatProperties(st.getPlayer().getPos(), true);
	p.Entities = [];
	p.Parties = [];
	p.TerrainTemplate = "tactical.plains";
	p.CombatID = "Player";
	::Const.World.Common.addUnitsToCombat(p.Entities, ::Const.World.Spawn.BanditRaiders, 60, ::World.FactionManager.getFactionOfType(::Const.FactionType.Bandits).getID());
	st.startScriptedCombat(p, true, true, true);
};
"#;

/// Boot a campaign and offer the battle; the autoplay engages.
fn start() -> Option<harness::GameEngine> {
	start_with("")
}

/// [`start`], running `prepare` (Squirrel) on the world map before the battle.
fn start_with(prepare: &str) -> Option<harness::GameEngine> {
	let dir = Path::new(harness::STEAM_DATA_DIR);
	if !dir.exists() {
		eprintln!("skipping: {} not present", dir.display());
		return None;
	}
	let engine = harness::open_install(dir).expect("open install");
	harness::boot_menu(&engine).expect("boot the menu");
	harness::prepare_new_campaign(&engine, "rust-engine-test").expect("prepare the campaign");
	harness::start_new_campaign(&engine).expect("startNewCampaign");
	harness::install_autoplay(&engine).expect("autoplay");
	engine.run_script(SETUP, "setup").expect("setup");
	engine.run_script(prepare, "prepare").expect("prepare");
	for frame in 0..120 {
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap_or_else(|e| panic!("world frame {frame}: {e}"));
	}
	engine.run_script("::__autoplayEngage = true; ::__offerBattle();", "engage").expect("engage");
	Some(engine)
}

fn result(engine: &harness::GameEngine) -> Option<i64> {
	SqTable::root(engine.vm()).get::<i64>("__lastCombatResult")
}

#[derive(Default, Debug)]
struct Played {
	waits: u32,
	moves: u32,
	skills: u32,
	ends: u32,
	/// Items taken out of a bag through the inventory.
	equipped: u32,
	/// Skills used, by id.
	used: Vec<String>,
	/// The most tiles burning at once.
	fire_tiles: i64,
}

/// Run frames until `done` holds (at most `max`).
fn frames_until(engine: &harness::GameEngine, max: u32, done: impl Fn(&harness::GameEngine) -> bool) -> bool {
	for frame in 0..max {
		if done(engine) {
			return true;
		}
		harness::run_frame(engine, 1000.0 / 60.0).unwrap_or_else(|e| panic!("frame {frame}: {e}"));
	}
	done(engine)
}

/// Open the inventory with its key, equip the acting brother's first bag item of type
/// `item_id`, close it again. Whether it got equipped.
fn take_from_bag(engine: &harness::GameEngine, item_id: &str) -> bool {
	let slot = player::acting_i64(
		engine,
		&format!(
			"(function() {{ for (local i = 0; i < e.getItems().getUnlockedBagSlots(); i++) {{ local it = e.getItems().getItemAtBagSlot(i); if (it != null && it.getID() == \"{item_id}\") return i; }} return -1; }})()"
		),
	)
	.unwrap()
	.unwrap();
	if slot < 0 {
		return false;
	}
	player::press_key(engine, player::KEY_C).expect("open the inventory");
	assert!(frames_until(engine, 30, |e| player::inventory_open(e).unwrap()), "the inventory didn't open");
	let equipped = player::equip_bag_item(engine, slot).expect("equip");
	player::press_key(engine, player::KEY_C).expect("close the inventory");
	assert!(frames_until(engine, 30, |e| player::ready(e).unwrap()), "the battle didn't come back");
	equipped
}

/// Play the company's turns until the battle ends. With `bag_item`, each brother first
/// takes that item out of its bag (once).
fn play(engine: &harness::GameEngine, bag_item: Option<&str>) -> Played {
	let mut played = Played::default();
	let mut waited = std::collections::HashSet::new();
	let mut unpacked = std::collections::HashSet::new();
	for frame in 0..60_000 {
		harness::run_frame(engine, 1000.0 / 60.0).unwrap_or_else(|e| panic!("battle frame {frame}: {e}"));
		if result(engine).is_some() {
			break;
		}
		let burning = player::eval_i64(engine, "(\"Entities\" in ::Tactical) ? ::Tactical.Entities.m.TileEffects.len() : 0").unwrap();
		played.fire_tiles = played.fire_tiles.max(burning.unwrap_or(0));
		if !player::ready(engine).expect("ready") {
			continue;
		}
		let name = player::eval_string(engine, "::Tactical.TurnSequenceBar.getActiveEntity().getName()").unwrap().unwrap();
		let round = player::eval_i64(engine, "::Tactical.TurnSequenceBar.getCurrentRound()").unwrap().unwrap();
		if let Some(item) = bag_item {
			if unpacked.insert(name.clone()) && take_from_bag(engine, item) {
				played.equipped += 1;
				continue;
			}
		}
		if round == 1 && waited.insert(name.clone()) && player::acting_i64(engine, "e.isAbleToWait() ? 1 : 0").unwrap() == Some(1) {
			player::wait_turn(engine).expect("wait");
			played.waits += 1;
			continue;
		}
		let decision = player::eval_string(engine, "::__decide()").unwrap().unwrap();
		let words: Vec<&str> = decision.split(' ').collect();
		match words[0] {
			"skill" => {
				let (x, y) = (words[2].parse().unwrap(), words[3].parse().unwrap());
				player::use_skill(engine, words[1], Some((x, y))).expect("use skill");
				played.skills += 1;
				played.used.push(words[1].to_string());
			}
			"move" if played.moves < 200 => {
				let (x, y) = (words[1].parse().unwrap(), words[2].parse().unwrap());
				if !player::move_to(engine, x, y).expect("move") {
					player::end_turn(engine).expect("end turn");
					played.ends += 1;
				}
				played.moves += 1;
			}
			_ => {
				player::end_turn(engine).expect("end turn");
				played.ends += 1;
			}
		}
	}
	played
}

#[test]
fn the_player_fights_a_battle_through_its_input() {
	let Some(engine) = start() else { return };
	let played = play(&engine, None);
	let res = result(&engine).unwrap_or_else(|| panic!("the battle didn't end ({played:?})"));
	eprintln!("result {res}, {played:?}");
	assert!(played.waits >= 1 && played.moves >= 1 && played.skills >= 1 && played.ends >= 1, "{played:?}");
	// Won (the enemy fled or died) or lost (the company died): fought to the end.
	assert!([1, 2, 4].contains(&res), "result {res}");
}

#[test]
fn the_player_takes_fire_bombs_from_the_bags_and_throws_them() {
	let Some(engine) = start_with(
		"foreach (bro in ::World.getPlayerRoster().getAll()) bro.getItems().addToBag(::new(\"scripts/items/tools/fire_bomb_item\"));
		 ::__prefer = [\"actives.throw_fire_bomb\"];",
	) else {
		return;
	};
	let played = play(&engine, Some("weapon.fire_bomb"));
	let res = result(&engine).unwrap_or_else(|| panic!("the battle didn't end ({played:?})"));
	eprintln!("result {res}, {played:?}");
	assert!(played.equipped >= 1, "no bomb taken out of a bag: {played:?}");
	assert!(played.used.iter().any(|s| s == "actives.throw_fire_bomb"), "no bomb thrown: {played:?}");
	// A bomb sets its tile and the six around it on fire (fewer at the map's edge).
	assert!(played.fire_tiles >= 4, "no fire: {played:?}");
	assert!([1, 2, 4].contains(&res), "result {res}");
}

#[test]
fn the_player_retreats() {
	let Some(engine) = start() else { return };
	let mut retreated = false;
	for frame in 0..30_000 {
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap_or_else(|e| panic!("battle frame {frame}: {e}"));
		if result(&engine).is_some() {
			break;
		}
		if !retreated && player::ready(&engine).expect("ready") {
			player::retreat(&engine).expect("retreat");
			retreated = true;
		}
	}
	assert!(retreated, "no player turn came");
	assert_eq!(result(&engine), Some(3), "PlayerRetreated");
}
