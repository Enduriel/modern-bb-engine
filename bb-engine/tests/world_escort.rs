//! Escorting a caravan hides the company on the map, as in the original: the escort
//! contract's update puts the company on the caravan and calls `setVisible(false)` every
//! frame, and an entity made invisible isn't drawn at all (its sprites and labels), in
//! sight or not. Cancelling the contract shows it again (`setVisible(true)` in
//! `onClear`). The original's frames: `parity/frames/escort.nut`.

use std::path::Path;

use bb_engine::harness::{self, GameEngine};

fn script_int(engine: &GameEngine, code: &str) -> i64 {
	engine.run_script(&format!("::__v <- {code};"), "value").expect(code);
	squirrel::SqTable::root(engine.vm()).get::<i64>("__v").expect(code)
}

fn script_bool(engine: &GameEngine, code: &str) -> bool {
	engine.run_script(&format!("::__v <- {code};"), "value").expect(code);
	squirrel::SqTable::root(engine.vm()).get::<bool>("__v").expect(code)
}

/// Whether the entity is drawn at all, and whether it has a sprite to draw.
fn drawn(engine: &GameEngine, id: i64) -> (bool, bool) {
	let scene = engine.world_scene();
	let e = scene.entities.iter().find(|e| e.id == id).expect("the entity");
	(e.shown, e.sprites.iter().any(|s| s.visible && !s.brush.is_empty()))
}

fn frames(engine: &GameEngine, n: usize) {
	for _ in 0..n {
		harness::run_frame(engine, 1000.0 / 60.0).expect("frame");
	}
}

#[test]
fn the_company_is_hidden_while_it_escorts_a_caravan() {
	let dir = Path::new(harness::STEAM_DATA_DIR);
	if !dir.exists() {
		harness::skip(format!("{} not present", dir.display()));
		return;
	}
	let engine = harness::open_install(dir).expect("open install");
	harness::boot_menu(&engine).expect("boot the menu");
	harness::prepare_new_campaign(&engine, "bbuitest").expect("prepare the campaign");
	harness::start_new_campaign(&engine).expect("startNewCampaign");
	engine.run_script("::World.State.m.IsAutosaving = false;", "no autosave").expect("autosave off");
	harness::unpause_world(&engine).expect("unpause");
	frames(&engine, 30);

	let player = script_int(&engine, "::World.State.getPlayer().getID()");
	assert_eq!(drawn(&engine, player), (true, true), "the company is drawn before the escort");

	// An escort contract taken and accepted as the town screen's buttons do it
	// (escort_caravan_action.onExecute, then start and the Overview's "I accept").
	engine
		.run_script(
			r#"
			local factions = ::World.FactionManager.getFactionsOfType(::Const.FactionType.Settlement).slice(0);
			factions.sort(function ( _a, _b ) { return _a.getID() <=> _b.getID(); });
			::__contract <- null;
			foreach( f in factions )
			{
				if (f.getSettlements().len() == 0) continue;
				local c = ::new("scripts/contracts/contracts/escort_caravan_contract");
				c.setFaction(f.getID());
				c.setEmployerID(f.getRandomCharacter().getID());
				c.setHome(f.getSettlements()[0]);
				c.setOrigin(f.getSettlements()[0]);
				c.setup();
				if (c.isValid()) { ::__contract = c; break; }
			}
			::World.Contracts.addContract(::__contract);
			::World.State.getPlayer().setPos(::__contract.m.Home.getPos());
			::__contract.start();
			::__contract.setState("Running");
			::World.State.setPause(false);
			"#,
			"escort",
		)
		.expect("take the escort contract");
	assert!(script_bool(&engine, "::World.Contracts.getActiveContract() == ::__contract"), "the contract is active");

	// Riding along: hidden on the caravan, every frame.
	for second in 0..3 {
		frames(&engine, 60);
		assert!(!script_bool(&engine, "::World.State.getPlayer().isVisible()"), "isVisible() after {second} s");
		assert!(
			script_bool(&engine, "::World.State.getEscortedEntity() != null && !::World.State.getEscortedEntity().isNull()"),
			"escorting after {second} s"
		);
		let (shown, _) = drawn(&engine, player);
		assert!(!shown, "the company isn't drawn while it escorts ({second} s)");
	}
	let caravan = script_int(&engine, "::__contract.m.Caravan.getID()");
	assert!(drawn(&engine, caravan).0, "the caravan is drawn");

	// Cancelled: shown again.
	engine.run_script("::World.Contracts.removeContract(::__contract);", "cancel").expect("cancel");
	frames(&engine, 30);
	assert!(script_bool(&engine, "::World.State.getPlayer().isVisible()"), "visible after the escort");
	assert_eq!(drawn(&engine, player), (true, true), "the company is drawn after the escort");
}
