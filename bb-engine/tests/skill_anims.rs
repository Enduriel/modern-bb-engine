//! Skills' animations on a unit's sprites in a battle, frame by frame, as the scripts
//! drive them from `onRender` (actor.nut) and the original draws them:
//!
//! - Shieldwall raises the shield: `shield_icon` slides from its place to
//!   `Const.Items.Default.RaiseShieldOffset` (0, 10) over `RaiseShieldDuration` (0.15 s
//!   of virtual time) through `moveSpriteOffset`, and back over `LowerShieldDuration`
//!   (0.4 s) once the effect goes.
//! - Spearwall lowers the weapon: `arms_icon` turns to −33° (one-handed) over
//!   `LowerWeaponDuration` and back over `RaiseWeaponDuration`.
//!
//! The renderer sees each in the battle's scene.
//!
//! Skips without the Steam install.

use std::path::Path;

use bb_engine::{harness, tactical_player as player};

/// A unit of the player's, with a heater shield in its off hand and nothing two-handed.
const SETUP: &str = r#"
	local u = ::Tactical.Entities.getInstancesOfFaction(::Const.Faction.Player)[0];
	local items = u.getItems();
	local off = items.getItemAtSlot(::Const.ItemSlot.Offhand);
	if (off != null) items.unequip(off);
	local main = items.getItemAtSlot(::Const.ItemSlot.Mainhand);
	if (main != null && main.isItemType(::Const.Items.ItemType.TwoHanded)) items.unequip(main);
	items.equip(::new("scripts/items/shields/heater_shield"));
	::__u <- u;
"#;

/// A battle on the map, with the unit set up ([`SETUP`]); `None` without the Steam
/// install.
fn battle() -> Option<harness::GameEngine> {
	let dir = Path::new(harness::STEAM_DATA_DIR);
	if !dir.exists() {
		bb_engine::harness::skip(format!("{} not present", dir.display()));
		return None;
	}
	let engine = harness::open_install(dir).expect("open install");
	harness::boot_menu(&engine).expect("boot the menu");
	harness::prepare_new_campaign(&engine, "rust-engine-test").expect("prepare the campaign");
	harness::start_new_campaign(&engine).expect("startNewCampaign");
	harness::unpause_world(&engine).expect("unpause");
	for frame in 0..60 {
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap_or_else(|e| panic!("world frame {frame}: {e}"));
	}
	harness::engage_nearest_hostile(&engine).expect("engage");
	let in_battle = "(\"State\" in ::Tactical) && ::Tactical.State != null && !::LoadingScreen.isVisible() && ::Tactical.Entities.getInstancesOfFaction(::Const.Faction.Player).len() > 0";
	for _ in 0..3_000 {
		if player::eval_i64(&engine, &format!("({in_battle}) ? 1 : 0")).unwrap() == Some(1) {
			break;
		}
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap();
	}
	engine.run_script(SETUP, "setup").expect("a unit with a shield");
	Some(engine)
}

/// The unit's sprite `name` in the battle's scene.
fn sprite(engine: &harness::GameEngine, name: &str) -> squirrel_bb::scene::tactical::SceneTacticalSprite {
	let id = player::eval_i64(engine, "::__u.getID()").unwrap().unwrap();
	let scene = engine.tactical_scene();
	let unit = scene.entities.iter().find(|e| e.id == id).expect("the unit in the scene");
	unit.sprites.iter().find(|s| s.name == name).unwrap_or_else(|| panic!("a {name} sprite")).clone()
}

/// The shield's offset as the scripts read it back, and virtual seconds.
fn shield(engine: &harness::GameEngine) -> (f64, f64, f64) {
	let s = player::eval_string(
		engine,
		"(function() { local o = ::__u.getSpriteOffset(\"shield_icon\"); return o.X + \" \" + o.Y + \" \" + ::Time.getVirtualTimeF(); })()",
	)
	.unwrap()
	.unwrap();
	let v: Vec<f64> = s.split(' ').map(|x| x.parse().unwrap()).collect();
	(v[0], v[1], v[2])
}

/// Frames for `seconds` of virtual time; the shield's offset after each.
fn frames(engine: &harness::GameEngine, seconds: f64) -> Vec<(f64, f64, f64)> {
	let start = shield(engine).2;
	let mut out = Vec::new();
	for _ in 0..600 {
		harness::run_frame(engine, 1000.0 / 60.0).unwrap();
		let s = shield(engine);
		out.push(s);
		if s.2 - start > seconds {
			break;
		}
	}
	out
}

fn flag(engine: &harness::GameEngine, name: &str) -> bool {
	player::eval_i64(engine, &format!("::__u.m.{name} ? 1 : 0")).unwrap() == Some(1)
}

#[test]
fn shieldwall_raises_the_shield_and_lowers_it_after() {
	let Some(engine) = battle() else { return };
	assert_eq!(sprite(&engine, "shield_icon").offset, (0.0, 0.0), "the shield down at first");
	assert!(sprite(&engine, "shield_icon").visible, "the shield shows");

	engine
		.run_script("::__u.getSkills().add(::new(\"scripts/skills/effects/shieldwall_effect\"));", "shieldwall")
		.expect("shieldwall");
	assert!(flag(&engine, "IsRaisingShield"), "raising");
	let start = shield(&engine).2;
	let up = frames(&engine, 0.5);
	// There after RaiseShieldDuration, and not before (as the original: 0.15 s, its
	// probe's samples a frame behind).
	let at = up.iter().find(|s| s.1 == 10.0).map(|s| s.2 - start).expect("raised");
	assert!((0.15..0.2).contains(&at), "raised {at:.3} s in");
	// Slid up, not jumped: some frame in between (0.15 s is 9 frames).
	assert!(up.iter().any(|&(x, y, _)| x == 0.0 && y > 0.0 && y < 10.0), "on its way up: {up:?}");
	assert!(up.windows(2).all(|w| w[1].1 >= w[0].1), "only up: {up:?}");
	assert_eq!(up.last().map(|s| (s.0, s.1)), Some((0.0, 10.0)), "raised by RaiseShieldOffset");
	assert_eq!(sprite(&engine, "shield_icon").offset, (0.0, 10.0), "the renderer sees it raised");
	assert!(!flag(&engine, "IsRaisingShield"), "the raise is over");

	// Another appearance update while raised (the shield's offset isn't 0 now) starts no
	// new raise.
	engine.run_script("::__u.getItems().updateAppearance();", "update").unwrap();
	assert!(!flag(&engine, "IsRaisingShield"), "no second raise");

	engine.run_script("::__u.getSkills().removeByID(\"effects.shieldwall\");", "down").expect("down");
	assert!(flag(&engine, "IsLoweringShield"), "lowering");
	let start = shield(&engine).2;
	let down = frames(&engine, 0.8);
	let at = down.iter().find(|s| s.1 == 0.0).map(|s| s.2 - start).expect("lowered");
	assert!((0.4..0.45).contains(&at), "lowered {at:.3} s in (LowerShieldDuration 0.4 s)");
	assert!(down.iter().any(|&(_, y, _)| y > 0.0 && y < 10.0), "on its way down: {down:?}");
	assert!(down.windows(2).all(|w| w[1].1 <= w[0].1), "only down: {down:?}");
	assert_eq!(down.last().map(|s| (s.0, s.1)), Some((0.0, 0.0)), "back down");
	assert_eq!(sprite(&engine, "shield_icon").offset, (0.0, 0.0), "the renderer sees it down");
	assert!(!flag(&engine, "IsLoweringShield"), "the lowering is over");
}

#[test]
fn spearwall_lowers_the_weapon_and_raises_it_after() {
	let Some(engine) = battle() else { return };
	engine
		.run_script(
			"local items = ::__u.getItems();
			 items.unequip(items.getItemAtSlot(::Const.ItemSlot.Offhand));
			 local main = items.getItemAtSlot(::Const.ItemSlot.Mainhand);
			 if (main != null) items.unequip(main);
			 items.equip(::new(\"scripts/items/weapons/militia_spear\"));",
			"spear",
		)
		.expect("a spear");
	assert_eq!(sprite(&engine, "arms_icon").rotation, 0.0, "upright at first");
	engine
		.run_script("::__u.getSkills().add(::new(\"scripts/skills/effects/spearwall_effect\"));", "spearwall")
		.expect("spearwall");
	frames(&engine, 0.6);
	assert_eq!(sprite(&engine, "arms_icon").rotation, -33.0, "lowered, one-handed");
	engine.run_script("::__u.getSkills().removeByID(\"effects.spearwall\");", "up").expect("up");
	frames(&engine, 0.6);
	assert_eq!(sprite(&engine, "arms_icon").rotation, 0.0, "upright again");
}
