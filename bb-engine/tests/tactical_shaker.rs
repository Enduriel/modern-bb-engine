//! The shaker (`Tactical.getShaker()`) in a battle, frame by frame, as the original's
//! (`0x4b6cc0`, `0x4b7450`): a hit pushes the unit away from the attacker's tile and back
//! (7 × 6 units, 0.08 s out, 0.4 s back, on virtual time), flashing the hit sprites; a
//! dodge goes the same way turned 45° to one side; a shake on a shaking unit is ignored;
//! `cancel` puts the unit back at once. The renderer sees the moved unit and the flash in
//! the battle's scene.
//!
//! Skips without the Steam install.

use std::path::Path;

use bb_engine::{harness, tactical_player as player};

/// A unit of the player's with a tile next to it (its direction 0 or later).
const SETUP: &str = r#"
	local u = null, src = null;
	foreach (e in ::Tactical.Entities.getInstancesOfFaction(::Const.Faction.Player))
	{
		for (local d = 0; d < 6 && src == null; d++)
			if (e.getTile().hasNextTile(d)) src = e.getTile().getNextTile(d);
		if (src != null) { u = e; break; }
	}
	::__u <- u;
	::__src <- src;
	::__orig <- u.getPos();
"#;

/// The unit's offset from where it stood, and virtual seconds.
const SAMPLE: &str = r#"(function() {
	local p = ::__u.getPos();
	return (p.X - ::__orig.X) + " " + (p.Y - ::__orig.Y) + " " + ::Time.getVirtualTimeF();
})()"#;

fn sample(engine: &harness::GameEngine) -> (f64, f64, f64) {
	let s = player::eval_string(engine, SAMPLE).unwrap().unwrap();
	let v: Vec<f64> = s.split(' ').map(|x| x.parse().unwrap()).collect();
	(v[0], v[1], v[2])
}

/// The unit's flash on its body sprite in the scene: (highlight, factor).
fn flash(engine: &harness::GameEngine, id: i64) -> (u32, f32) {
	let scene = engine.tactical_scene();
	let unit = scene.entities.iter().find(|e| e.id == id).expect("the unit in the scene");
	let body = unit.sprites.iter().find(|s| s.name == "body").expect("a body sprite");
	(body.highlight, body.highlight_factor)
}

/// Frames until the shake has ended (a second of virtual time is plenty); every sample.
fn run(engine: &harness::GameEngine, id: i64) -> Vec<((f64, f64, f64), (u32, f32))> {
	let start = sample(engine).2;
	let mut out = Vec::new();
	for _ in 0..600 {
		harness::run_frame(engine, 1000.0 / 60.0).unwrap();
		let s = sample(engine);
		out.push((s, flash(engine, id)));
		if s.2 - start > 1.0 {
			break;
		}
	}
	out
}

/// A battle on the map, with a unit of the player's picked ([`SETUP`]); `None` without
/// the Steam install.
fn battle() -> Option<harness::GameEngine> {
	let dir = Path::new(harness::STEAM_DATA_DIR);
	if !dir.exists() {
		eprintln!("skipping: {} not present", dir.display());
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
	engine.run_script(SETUP, "setup").expect("a unit with a neighbour");
	Some(engine)
}

#[test]
fn units_shake_as_the_originals_do() {
	let Some(engine) = battle() else { return };
	let id = player::eval_i64(&engine, "::__u.getID()").unwrap().unwrap();
	let away = player::eval_string(&engine, "(::__orig.X - ::__src.Pos.X) + \" \" + (::__orig.Y - ::__src.Pos.Y)").unwrap().unwrap();
	let away: Vec<f64> = away.split(' ').map(|x| x.parse().unwrap()).collect();
	let len = away[0].hypot(away[1]);
	let away = (away[0] / len, away[1] / len);

	// A hit: away from the tile, 7 across and 6 up or down (7 when up), flashing the body
	// in the hit colour at up to half strength. Then a straight-up shake (type 7) right
	// after it is ignored.
	engine
		.run_script(
			"::Tactical.getShaker().shake(::__u, ::__src, 2, ::createColor(\"#ffffff\"), ::createColor(\"#600a0a\"), 0.5, 1.0, [\"body\"], 1.0);
			 ::Tactical.getShaker().shake(::__u, ::__src, 7);",
			"hit",
		)
		.expect("shake");
	let hit = run(&engine, id);
	let (furthest, at) = hit.iter().map(|((x, y, t), _)| (x.hypot(*y), *t)).fold((0.0, 0.0), |a, b| if b.0 > a.0 { b } else { a });
	let start = hit[0].0.2 - 1.0 / 60.0;
	let expected = (away.0 * 7.0).hypot(away.1 * if away.1 > 0.0 { 7.0 } else { 6.0 });
	assert!((furthest - expected).abs() < 1.0, "pushed {furthest:.2} out of {expected:.2}");
	assert!(at - start < 0.15, "furthest {:.3} s in (0.08 s out)", at - start);
	for ((x, y, _), _) in &hit {
		let along = x * away.0 + y * away.1;
		assert!(along >= -1e-3 && (x - along * away.0).hypot(y - along * away.1) < 0.05, "off the line away from the tile: ({x}, {y})");
	}
	let ((x, y, _), last_flash) = *hit.last().unwrap();
	assert!(x.hypot(y) < 1e-6, "back where it stood: ({x}, {y})");
	assert_eq!(last_flash, (0, 0.0), "the flash is over");
	let strongest = hit.iter().map(|(_, (_, f))| *f).fold(0.0, f32::max);
	assert!((strongest - 0.5).abs() < 0.1, "flashes at up to 0.5 ({strongest})");
	assert!(hit.iter().filter(|(_, (_, f))| *f > 0.0).all(|(_, (h, _))| *h == 0xff60_0a0a), "in the hit's colour");

	// A dodge: away turned 45° to one side.
	engine.run_script("::Tactical.getShaker().shake(::__u, ::__src, 4);", "dodge").expect("dodge");
	let dodge = run(&engine, id);
	let ((x, y, _), _) = dodge.iter().copied().max_by(|a, b| a.0.0.hypot(a.0.1).total_cmp(&b.0.0.hypot(b.0.1))).unwrap();
	// The direction before the amplitudes (7 across, 6 down or 7 up) scaled it.
	let (dx, dy) = (x / 7.0, y / if y > 0.0 { 7.0 } else { 6.0 });
	let cos = (dx * away.0 + dy * away.1) / dx.hypot(dy);
	assert!((cos - std::f64::consts::FRAC_1_SQRT_2).abs() < 0.05, "turned 45° from away (cos {cos:.3})");
	assert!(dodge.last().unwrap().0.0.hypot(dodge.last().unwrap().0.1) < 1e-6, "back where it stood");

	// Cancelled halfway out: back at once.
	engine.run_script("::Tactical.getShaker().shake(::__u, ::__src, 1);", "hop").expect("hop");
	for _ in 0..10 {
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap();
	}
	assert!(sample(&engine).1 > 1.0, "hopping up");
	engine.run_script("::Tactical.getShaker().cancel(::__u);", "cancel").expect("cancel");
	let (x, y, _) = sample(&engine);
	assert!(x.hypot(y) < 1e-6, "cancelled: back where it stood ({x}, {y})");
}

/// The calling unit's colours as a whole: kept and put back (`isStoringColor` meanwhile,
/// which a kraken's tentacles wait on), faded to a colour and back to the kept one, its
/// alpha set and faded, a sprite's scale moved.
#[test]
fn units_keep_and_fade_their_colours() {
	let Some(engine) = battle() else { return };
	let id = player::eval_i64(&engine, "::__u.getID()").unwrap().unwrap();
	let body = |engine: &harness::GameEngine| {
		let scene = engine.tactical_scene();
		let unit = scene.entities.iter().find(|e| e.id == id).expect("the unit");
		unit.sprites.iter().find(|s| s.name == "body").expect("a body").color
	};
	let storing = |engine: &harness::GameEngine| player::eval_i64(engine, "::__u.isStoringColor() ? 1 : 0").unwrap() == Some(1);
	let before = body(&engine);
	assert!(!storing(&engine));
	engine.run_script("::__u.storeSpriteColors(); ::__u.fadeTo(::createColor(\"ffffff00\"), 0);", "store").unwrap();
	assert!(storing(&engine), "storing");
	assert_eq!(body(&engine), 0x00ff_ffff, "faded to clear white at once");
	engine.run_script("::__u.fadeToStoredColors(100);", "back").unwrap();
	for _ in 0..12 {
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap();
	}
	assert_eq!(body(&engine), before, "faded back to the kept colour");
	assert!(storing(&engine), "still storing until restored");
	engine.run_script("::__u.restoreSpriteColors();", "restore").unwrap();
	assert!(!storing(&engine), "restored");

	engine.run_script("::__u.setAlpha(0);", "alpha").unwrap();
	assert_eq!(body(&engine) >> 24, 0, "clear at once");
	engine.run_script("::__u.fadeIn(100);", "fade in").unwrap();
	for _ in 0..12 {
		harness::run_frame(&engine, 1000.0 / 60.0).unwrap();
	}
	assert_eq!(body(&engine) >> 24, 0xff, "faded in");

	let scaled = player::eval_string(
		&engine,
		"(function() { local t = ::Time.getVirtualTimeF(); local a = ::__u.moveSpriteScale(\"body\", 1.0, 2.0, 1.0, t - 0.5); \
		 local b = ::__u.getSprite(\"body\").Scale; local c = ::__u.moveSpriteScale(\"body\", 1.0, 2.0, 1.0, t - 2.0); \
		 return a + \" \" + b + \" \" + c + \" \" + ::__u.getSprite(\"body\").Scale; })()",
	)
	.unwrap()
	.unwrap();
	assert_eq!(scaled, "false 1.5 true 2", "halfway, then there");
}
