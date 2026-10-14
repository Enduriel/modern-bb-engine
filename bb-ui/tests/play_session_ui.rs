//! A long play session through the real UI, as a player plays one: a new campaign from
//! the main menu, then into the nearest town and its screens. The game's `ui/` runs in
//! the Chromium host (offscreen, headless) against the real scripts; each step clicks
//! the page as a player would and checks what it shows. Skips if there is no Steam
//! install.
//!
//! Run: `cargo test -p bb-ui --features cef --test play_session_ui`. With
//! `PLAY_SHOTS=<dir>`, a screenshot of the page and its visible buttons are saved at
//! each step.
//!
//! A plain `main` (`harness = false`): Chromium re-runs this binary for its child
//! processes, and CEF can only start once per process.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use bb_engine::harness::STEAM_DATA_DIR;
use bb_engine::tactical_player as player;
use bb_ui::host::{HostConfig, MouseButton, UiSession};

/// How long a step may take (a new campaign generates its world first).
const STEP: Duration = Duration::from_secs(120);
const SEED: &str = "bbuitest";
const SEED_FIELD: &str = ".new-campaign-menu-module input[maxlength=\"10\"]";

fn ensure(condition: bool, what: impl std::fmt::Display) -> Result<(), String> {
	if condition { Ok(()) } else { Err(what.to_string()) }
}

fn during<T>(what: &str, result: Result<T, String>) -> Result<T, String> {
	result.map_err(|error| format!("{what}: {error}"))
}

/// The session, and where its step shots go.
struct Play {
	session: UiSession,
	shots: Option<PathBuf>,
	step: std::cell::Cell<u32>,
}

impl Play {
	/// Record step `name`: a screenshot and the visible buttons, with `PLAY_SHOTS`.
	fn shot(&self, name: &str) {
		let Some(dir) = &self.shots else { return };
		let n = self.step.get() + 1;
		self.step.set(n);
		let _ = self.session.screenshot(&dir.join(format!("{n:02}-{name}.png")));
		let buttons = self.session.visible_buttons().unwrap_or_default();
		println!("[step {n:02} {name}] buttons: {buttons:?}");
	}

	fn script(&self, code: &str) -> Result<(), String> {
		self.session.engine().run_script(code, "play")
	}

	fn int(&self, expr: &str) -> Result<i64, String> {
		player::eval_i64(self.session.engine(), expr)?.ok_or_else(|| format!("no value: {expr}"))
	}

	fn text(&self, expr: &str) -> Result<String, String> {
		player::eval_string(self.session.engine(), expr)?.ok_or_else(|| format!("no value: {expr}"))
	}

	/// Run the game until the scripts' `cond` holds.
	fn until(&self, cond: &str, timeout: Duration) -> Result<(), String> {
		let deadline = std::time::Instant::now() + timeout;
		loop {
			if self.int(&format!("({cond}) ? 1 : 0"))? == 1 {
				return Ok(());
			}
			if std::time::Instant::now() > deadline {
				return Err(format!("still not true: {cond}"));
			}
			self.session.step(1000.0 / 60.0)?;
		}
	}
}

fn new_campaign(p: &Play) -> Result<(), String> {
	let s = &p.session;
	for step in ["New Campaign", "A New Company", "Next", "Next"] {
		during(&format!("clicking {step:?}"), s.click_when_ready(step, STEP))?;
	}
	during("typing the seed", s.fill_field(SEED_FIELD, SEED))?;
	during("clicking \"Start\"", s.click_when_ready("Start", STEP))?;
	during("waiting for the intro event", s.wait_for_text("Huzzah!", STEP))?;
	p.shot("intro");
	during("clicking \"Huzzah!\"", s.click_when_ready("Huzzah!", STEP))?;
	during("waiting for the world screen", s.wait_for_text("Day 1", STEP))?;
	p.shot("world");
	Ok(())
}

/// Walk the company to the nearest town and into it.
fn enter_town(p: &Play) -> Result<String, String> {
	p.script(
		r#"
		local player = ::World.State.getPlayer(), best = null, d = 0;
		foreach (s in ::World.EntityManager.getSettlements())
		{
			local dist = s.getTile().getDistanceTo(player.getTile());
			if (s.isEnterable() && (best == null || dist < d)) { best = s; d = dist; }
		}
		::__town <- best;
		::World.State.m.AutoEnterLocation = ::WeakTableRef(best);
		player.setDestination(best.getPos());
		::World.State.setPause(false);
		"#,
	)?;
	let town = p.text("::__town.getName()")?;
	during(
		&format!("travelling to {town}"),
		p.until("::World.State.m.LastEnteredTown != null && ::World.State.m.WorldTownScreen.isVisible()", STEP),
	)?;
	p.session.pump_until(Duration::from_secs(5), |s| !s.animating().unwrap_or(true));
	p.shot("town");
	Ok(town)
}

/// Where the first item of `owner` ("shop" or "stash") is on the page, and its index.
fn item_at(p: &Play, owner: &str) -> Result<(i32, i32, i64), String> {
	let found = p.session.eval(&format!(
		r#"(function () {{
			var owner = 'world-town-screen-shop-dialog-module.{owner}', hit = null;
			jQuery('.ui-control').each(function () {{
				var d = jQuery(this).data('item');
				if (hit === null && d && d.owner === owner && d.isEmpty === false && this.offsetParent !== null) {{
					var r = this.getBoundingClientRect();
					hit = [Math.round(r.left + r.width / 2), Math.round(r.top + r.height / 2), d.index];
				}}
			}});
			return hit;
		}})()"#
	))?;
	let v = found.as_array().filter(|a| a.len() == 3).ok_or_else(|| format!("no {owner} item on the page"))?;
	let n = |i: usize| v[i].as_i64().ok_or("not a number");
	Ok((n(0)? as i32, n(1)? as i32, n(2)?))
}

/// Run the game until the page has stopped animating and `ready` holds.
fn settled(p: &Play, ready: impl Fn(&Play) -> bool) -> Result<(), String> {
	let ok = p.session.pump_until(STEP, |s| !s.animating().unwrap_or(true) && ready(p));
	ok.then_some(()).ok_or_else(|| "the page didn't settle".to_string())
}

/// Right-click at `(x, y)`, as a player buys or sells with.
fn right_click(p: &Play, x: i32, y: i32) -> Result<(), String> {
	p.session.move_to(x, y)?;
	p.session.host().mouse_button(x, y, MouseButton::Right, true);
	p.session.host().mouse_button(x, y, MouseButton::Right, false);
	p.session.pump_until(Duration::from_millis(500), |_| false);
	Ok(())
}

/// Open the town's building `id` by clicking its picture.
fn open_building(p: &Play, id: &str) -> Result<(), String> {
	let slot = p.int(&format!(
		"(function() {{ local i = 0; foreach (b in ::__town.m.Buildings) {{ if (b == null) continue; if (b.getID() == \"{id}\") return i; i++; }} return -1; }})()"
	))?;
	ensure(slot >= 0, format!("no {id} in the town"))?;
	during(&format!("clicking the {id}"), p.session.click_when_ready(&format!("css:img.slot{slot}:not(.no-pointer-events)"), STEP))
}

/// The marketplace: buy the first item on sale, then sell the first item of the
/// company's, each with a right-click; the crowns move by the item's price.
fn market(p: &Play) -> Result<(), String> {
	open_building(p, "building.marketplace")?;
	during("waiting for the market's goods", settled(p, |p| item_at(p, "shop").is_ok()))?;
	p.shot("market");
	let money = || p.int("::World.Assets.getMoney()");
	let (x, y, idx) = item_at(p, "shop")?;
	let price = p.int(&format!("::World.State.getTownScreen().getShopDialogModule().getShop().getStash().getItemAtIndex({idx}).item.getBuyPrice()"))?;
	let name = p.text(&format!("::World.State.getTownScreen().getShopDialogModule().getShop().getStash().getItemAtIndex({idx}).item.getName()"))?;
	let before = money()?;
	right_click(p, x, y)?;
	during(&format!("buying {name} for {price}"), p.session.pump_until(Duration::from_secs(10), |_| money().is_ok_and(|m| m != before)).then_some(()).ok_or("the crowns didn't move".to_string()))?;
	ensure(money()? == before - price, format!("bought {name}: {before} - {price} != {}", money()?))?;
	p.shot("bought");
	println!("bought {name} for {price}");

	let (x, y, idx) = item_at(p, "stash")?;
	let price = p.int(&format!("::World.Assets.getStash().getItemAtIndex({idx}).item.getSellPrice()"))?;
	let name = p.text(&format!("::World.Assets.getStash().getItemAtIndex({idx}).item.getName()"))?;
	let before = money()?;
	right_click(p, x, y)?;
	during(&format!("selling {name} for {price}"), p.session.pump_until(Duration::from_secs(10), |_| money().is_ok_and(|m| m != before)).then_some(()).ok_or("the crowns didn't move".to_string()))?;
	ensure(money()? == before + price, format!("sold {name}: {before} + {price} != {}", money()?))?;
	p.shot("sold");
	println!("sold {name} for {price}");
	during("leaving the market", p.session.click_when_ready("Leave", STEP))?;
	Ok(())
}

/// The recruits: pick the first, try them out, then hire them; the crowns go down by
/// the try-out's cost, then the hiring cost, and the company grows by one.
fn hire(p: &Play) -> Result<(), String> {
	open_building(p, "building.crowd")?;
	during("waiting for the recruits", settled(p, |p| p.session.eval("jQuery('.list-entry:visible').length").ok().and_then(|n| n.as_i64()).unwrap_or(0) > 0))?;
	p.shot("recruits");
	during("picking a recruit", p.session.click_when_ready("css:.list-entry", STEP))?;
	let id = p.session.eval("jQuery('.list-entry.is-selected').data('entry').ID")?.as_i64().ok_or("no recruit selected")?;
	let recruit = format!("(function() {{ foreach (e in ::World.getRoster(::__town.getID()).getAll()) if (e.getID() == {id}) return e; return null; }})()");
	let name = p.text(&format!("{recruit}.getName()"))?;
	let money = || p.int("::World.Assets.getMoney()");
	let brothers = || p.int("::World.getPlayerRoster().getSize()");

	let tryout = p.int(&format!("{recruit}.getTryoutCost()"))?;
	let before = money()?;
	if p.int(&format!("{recruit}.isTryoutDone() ? 1 : 0"))? == 0 {
		during("trying the recruit out", p.session.click_when_ready("Try out", STEP))?;
		during("paying for the try-out", p.session.pump_until(Duration::from_secs(10), |_| money().is_ok_and(|m| m != before)).then_some(()).ok_or("the crowns didn't move".to_string()))?;
		ensure(money()? == before - tryout, format!("tried {name} out: {before} - {tryout} != {}", money()?))?;
	}
	let cost = p.int(&format!("::Math.ceil({recruit}.getHiringCost() * ::World.Assets.m.HiringCostMult)"))?;
	let (before, company) = (money()?, brothers()?);
	during("hiring the recruit", p.session.click_when_ready("Hire", STEP))?;
	during("paying for the hire", p.session.pump_until(Duration::from_secs(10), |_| brothers().is_ok_and(|b| b == company + 1)).then_some(()).ok_or("the company didn't grow".to_string()))?;
	ensure(money()? == before - cost, format!("hired {name}: {before} - {cost} != {}", money()?))?;
	p.shot("hired");
	println!("hired {name} for {cost} (try-out {tryout})");
	during("leaving the recruits", p.session.click_when_ready("Leave", STEP))?;
	Ok(())
}

/// A contract: open the town's first and talk it through to an agreement, as a player
/// does: the first option carries a negotiation on, until the offer and then the
/// overview can be accepted. The advance is paid on taking it.
fn contract(p: &Play) -> Result<(), String> {
	let money = || p.int("::World.Assets.getMoney()");
	let start = money()?;
	// One whose target the map marks: a place to clear or a camp to destroy.
	let index = p.int(r#"(function() {
		local wanted = ["contract.restore_location", "contract.drive_away_bandits", "contract.drive_away_barbarians",
			"contract.drive_away_nomads", "contract.destroy_goblin_camp", "contract.destroy_orc_camp",
			"contract.raze_attached_location", "contract.root_out_undead", "contract.investigate_cemetery"];
		foreach (i, c in ::__town.getContracts()) if (wanted.find(c.getType()) != null) return i;
		return 0;
	})()"#)?;
	during("opening a contract", p.session.click_when_ready(&format!("css:img.is-contract.contract{index}"), STEP))?;
	let mut advance = 0;
	for round in 0..10 {
		if p.int("::World.Contracts.getActiveContract() != null ? 1 : 0")? == 1 {
			break;
		}
		during("waiting for the next screen", settled(p, |p| p.session.eval("jQuery('[id^=\"button-\"] .ui-control:visible').length").ok().and_then(|n| n.as_i64()).unwrap_or(0) > 0))?;
		p.shot(&format!("contract-{round}"));
		if p.session.shows_text("I accept your offer.")? {
			advance = p.int("::World.Contracts.m.LastShown.getPayment().getInAdvance()")?;
			during("accepting the offer", p.session.click_when_ready("I accept your offer.", STEP))?;
		} else if p.session.shows_text("I accept this contract.")? {
			during("accepting the contract", p.session.click_when_ready("I accept this contract.", STEP))?;
		} else {
			during("carrying on", p.session.click_when_ready("css:[id^=\"button-\"] .ui-control", STEP))?;
		}
		p.session.pump_until(Duration::from_millis(500), |_| false);
	}
	during("taking the contract", p.until("::World.Contracts.getActiveContract() != null", STEP))?;
	ensure(money()? == start + advance, format!("the advance: {start} + {advance} != {}", money()?))?;
	let name = p.text("::World.Contracts.getActiveContract().getName()")?;
	println!("took {name}, {advance} in advance");
	Ok(())
}

/// What the session faces at a moment.
const FACING: &str = r#"(function() {
	if (("State" in ::Tactical) && ::Tactical.State != null)
	{
		local st = ::Tactical.State;
		if (!st.isBattleEnded()) return "battle";
		local r = st.m.TacticalCombatResultScreen;
		return r != null && r.isVisible() && !r.isAnimating() ? "results" : "wait";
	}
	if (!("State" in ::World) || ::World.State == null || ::LoadingScreen.isVisible()) return "wait";
	// A battle on its way in (the tactical state is added, its map not yet made).
	if (::RootState.get("TacticalFromWorldState") != null) return "wait";
	local w = ::World.State;
	local ev = w.getEventScreen();
	if (ev != null && ev.isVisible()) return ev.isAnimating() ? "wait" : "event";
	local d = w.m.CombatDialog;
	if (d != null && d.isVisible()) return d.isAnimating() ? "wait" : "combat";
	if (w.m.WorldTownScreen.isVisible()) return w.m.WorldTownScreen.isAnimating() ? "wait" : "town";
	if (::World.Contracts.getActiveContract() == null) return "done";
	return "map";
})()"#;

/// Head for what the map marks (the contract's selection marker), entering it.
const FOLLOW_MARKER: &str = r#"
	local player = ::World.State.getPlayer(), target = null;
	foreach (e in ::World.getAllEntitiesAtPos(::createVec(0, 0), 10000000.0))
		if (e.hasSprite("selection") && e.getSprite("selection").Visible) { target = e; break; }
	if (target != null && (!("__heading" in getroottable()) || ::__heading != target.getID()))
	{
		::__heading <- target.getID();
		::World.State.m.AutoEnterLocation = ::WeakTableRef(target);
		player.setDestination(target.getPos());
	}
	::World.State.setPause(false);
"#;

/// Answer what the session faces (see [`FACING`]) once, as a player does: an event
/// takes its first option, the combat dialog is engaged, a battle runs a frame with the
/// AI playing the company (the scripted player's `__autoplayFight`), the result screen
/// goes on to the loot, takes it all and leaves, a town is left, and on the map the
/// company heads for what the map marks. Counts the battles begun.
fn answer(p: &Play, facing: &str, last: &str, battles: &mut u32) -> Result<(), String> {
	if facing != last && facing != "wait" && facing != "battle" {
		p.shot(facing);
	}
	if facing != "map" && facing != "wait" {
		// Head off anew once back on the map.
		p.script("::__heading <- null;")?;
	}
	match facing {
		"battle" => {
			if last != "battle" {
				*battles += 1;
				p.shot("battle");
			}
			p.script("::__autoplay();")?;
			p.session.step(1000.0 / 60.0)?;
		}
		"results" => {
			// After a win, "Continue" goes on to the loot (take it all) and then leaves;
			// after a retreat or a loss there's nothing to take, and it reads "Leave".
			if p.session.shows_text("Continue")? {
				during("to the loot", p.session.click_when_ready("Continue", STEP))?;
				settled(p, |_| true)?;
				if p.session.eval("jQuery('.loot-all-button:visible').length")?.as_i64() == Some(1) {
					during("looting all", p.session.click_when_ready("css:.loot-all-button", STEP))?;
					p.session.pump_until(Duration::from_secs(1), |_| false);
					p.shot("looted");
				}
				during("leaving the battle", p.session.click_when_ready("Continue", STEP))?;
			} else {
				during("leaving the battle", p.session.click_when_ready("Leave", STEP))?;
			}
			p.until("!((\"State\" in ::Tactical) && ::Tactical.State != null)", STEP)?;
		}
		"event" => during("answering an event", p.session.click_when_ready("css:[id^=\"button-\"] .ui-control", STEP))?,
		"combat" => {
			// "Engage!" when the company attacks, "To Arms!" when it is attacked.
			let button = if p.session.shows_text("To Arms!")? { "To Arms!" } else { "Engage!" };
			during("engaging", p.session.click_when_ready(button, STEP))?;
			// Once per offer, as a player clicks once: until the dialog goes.
			let gone = "!::World.State.m.CombatDialog.isVisible() || ::LoadingScreen.isVisible() || ::RootState.get(\"TacticalFromWorldState\") != null";
			during("leaving for the battle", p.until(gone, STEP))?;
		}
		"town" => during("leaving the town", p.session.click_when_ready("Leave", STEP))?,
		"map" => {
			p.script(FOLLOW_MARKER)?;
			p.session.step(1000.0 / 60.0)?;
		}
		_ => p.session.step(1000.0 / 60.0)?,
	}
	Ok(())
}

/// Carry the contract out: follow the map's marker to its target and back, answering
/// what comes up (see [`answer`]). Ends when the contract is paid.
fn adventure(p: &Play) -> Result<(), String> {
	bb_engine::harness::install_autoplay(p.session.engine())?;
	p.script("::__autoplayFight = true;")?;
	during("leaving the town", p.session.click_when_ready("Leave", STEP))?;
	let money = || p.int("::World.Assets.getMoney()");
	let start = money()?;
	let reward = p.int("::World.Contracts.getActiveContract().getPayment().getOnCompletion()")?;
	let deadline = std::time::Instant::now() + Duration::from_secs(900);
	let (mut battles, mut last) = (0, String::new());
	loop {
		ensure(std::time::Instant::now() < deadline, format!("the contract took too long (facing {last})"))?;
		let facing = p.text(FACING)?;
		if facing == "done" {
			break;
		}
		answer(p, &facing, &last, &mut battles)?;
		last = facing;
	}
	ensure(money()? >= start + reward, format!("paid: {start} + {reward} > {}", money()?))?;
	println!("contract done after {battles} battle(s): paid {reward}");
	Ok(())
}

/// A battle: the company meets the nearest hostile party, engages through the combat
/// dialog, fights it out (the AI playing the company), and takes the loot; back on the
/// map, the stash holds what was looted.
fn battle(p: &Play) -> Result<(), String> {
	p.script(
		r#"
		local player = ::World.State.getPlayer(), target = null, best = 0;
		foreach (e in ::World.getAllEntitiesAtPos(player.getPos(), 100000.0))
		{
			if (!e.isParty() || e.getID() == player.getID() || e.isAlliedWithPlayer() || e.getTroops().len() == 0) continue;
			local d = e.getDistanceTo(player);
			if (target == null || d < best) { target = e; best = d; }
		}
		if (target == null) throw "no hostile party to fight";
		::__foe <- target.getName();
		local tp = target.getPos();
		player.setPos(::createVec(tp.X - 30, tp.Y));
		::World.setPlayerPos(player.getPos());
		::World.State.showCombatDialog(true, true, true, null, player.getPos());
		"#,
	)?;
	let foe = p.text("::__foe")?;
	let stash = || p.int("::World.Assets.getStash().getNumberOfFilledSlots()");
	let before = stash()?;
	let deadline = std::time::Instant::now() + Duration::from_secs(600);
	let (mut battles, mut last) = (0, String::new());
	let mut waited = 0;
	loop {
		ensure(std::time::Instant::now() < deadline, format!("the battle took too long (facing {last})"))?;
		let facing = p.text(FACING)?;
		waited = if battles > 0 && facing == "wait" { waited + 1 } else { 0 };
		ensure(waited < 1800, "after the battle, still waiting 30 s on")?;
		if battles > 0 && (facing == "map" || facing == "done") {
			break;
		}
		answer(p, &facing, &last, &mut battles)?;
		last = facing;
	}
	let result = p.int("::Tactical.Entities.getCombatResult()").unwrap_or(-1);
	println!("fought {foe}: stash {before} -> {} (result {result})", stash()?);
	Ok(())
}

fn play(p: &Play) -> Result<(), String> {
	new_campaign(p)?;
	let town = enter_town(p)?;
	println!("entered {town}");
	market(p)?;
	hire(p)?;
	contract(p)?;
	adventure(p)?;
	battle(p)?;
	let problems: Vec<String> = p
		.session
		.log()
		.into_iter()
		.filter(|line| line.level == "error" || line.level == "bridge")
		.map(|line| format!("[{}] {}", line.level, line.text))
		.collect();
	ensure(problems.is_empty(), format!("page or bridge errors:\n{}", problems.join("\n")))
}

/// The stubs the scripts called this session, and the natives the original binds that
/// this engine lacks (the parity suite's list): what the scripts ask for that this
/// engine ignores. Printed, and saved next to the shots with `PLAY_SHOTS`.
fn stub_report(p: &Play) {
	let calls = p.session.engine().stub_calls();
	let mut report = format!("Stub natives the scripts called ({}):\n", calls.len());
	for (name, count) in &calls {
		report += &format!("  {count:>8}  {name}\n");
	}
	let missing = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parity/missing_natives.txt");
	let lacking: Vec<String> = std::fs::read_to_string(&missing)
		.unwrap_or_default()
		.lines()
		.filter(|l| !l.trim_start().starts_with('#') && !l.trim().is_empty())
		.map(str::to_string)
		.collect();
	report += &format!("Natives the original binds that this engine lacks ({}, parity/missing_natives.txt):\n", lacking.len());
	for line in &lacking {
		report += &format!("  {line}\n");
	}
	println!("{report}");
	if let Some(dir) = &p.shots {
		let _ = std::fs::write(dir.join("stubs.txt"), &report);
	}
}

fn run(data_dir: &Path) -> Result<(), String> {
	let session = UiSession::start(data_dir, HostConfig::default(), false)?;
	during("booting the menu", session.boot_menu())?;
	let shots = std::env::var_os("PLAY_SHOTS").map(PathBuf::from);
	if let Some(dir) = &shots {
		let _ = std::fs::create_dir_all(dir);
	}
	let p = Play { session, shots, step: std::cell::Cell::new(0) };
	let played = play(&p);
	stub_report(&p);
	played.map_err(|error| {
		let sq = p.text(r#"(function() {
			local l = ::LoadingScreen;
			return "loading screen visible " + l.m.Visible + " animating " + l.m.Animating
				+ "; world state " + (("State" in ::World) && ::World.State != null ? "visible " + ::World.State.isVisible() + " screen " + ::World.State.m.WorldScreen.isVisible() : "none")
				+ "; tactical " + (("State" in ::Tactical) && ::Tactical.State != null ? (function(st) {
					return "ended " + st.m.IsBattleEnded + " finishable " + st.m.IsGameFinishable + " exiting " + st.m.IsExitingToMenu
						+ " backsteps " + st.m.MenuStack.hasBacksteps() + " results " + st.m.TacticalCombatResultScreen.isVisible()
						+ " dialog " + st.m.TacticalDialogScreen.isVisible() + " characters " + st.m.CharacterScreen.isVisible()
						+ " result " + ::Tactical.Entities.getCombatResult();
				})(::Tactical.State) : "none");
		})()"#).unwrap_or_else(|e| e);
		let js = p.session.eval(r#"(function () {
			var out = [];
			jQuery('body > div, body > div > div').each(function () {
				var e = jQuery(this);
				if (e.css('display') === 'none') return;
				var calls = (jQuery.Velocity && jQuery.Velocity.State) ? jQuery.Velocity.State.calls.filter(function (c) { return !!c; }).length : -1;
				out.push((this.className || this.tagName) + ' opacity ' + e.css('opacity') + (e.is(':animated') ? ' ANIMATING' : ''));
			});
			var calls = (jQuery.Velocity && jQuery.Velocity.State) ? jQuery.Velocity.State.calls.filter(function (c) { return !!c; }).length : -1;
			return 'velocity calls in flight ' + calls + '; ' + out.join(' | ');
		})()"#).map(|v| v.to_text()).unwrap_or_else(|e| e);
		// Whether the page still gets animation frames, and Velocity still ticks.
		let _ = p.session.eval("window.__raf = 0; (function f() { window.__raf++; requestAnimationFrame(f); })(); window.__timer = 0; setInterval(function () { window.__timer++; }, 10); 0");
		p.session.pump_until(Duration::from_secs(1), |_| false);
		let frames = p.session.eval("'rAF ' + window.__raf + ', timer ' + window.__timer + ', hidden ' + document.hidden + ', velocity ticking ' + (jQuery.Velocity && jQuery.Velocity.State.isTicking)").map(|v| v.to_text()).unwrap_or_else(|e| e);
		eprintln!("DIAG page over 1 s: {frames}");
		// A fresh animation restarts Velocity's ticking: do the stuck ones finish then?
		let _ = p.session.eval("jQuery('<div/>').appendTo('body').velocity({ opacity: 0 }, { duration: 10 }); 0");
		p.session.pump_until(Duration::from_secs(2), |_| false);
		let after = p.session.eval("'calls in flight ' + jQuery.Velocity.State.calls.filter(function (c) { return !!c; }).length + ', result screen opacity ' + jQuery('.tactical-combat-result-screen').css('opacity')").map(|v| v.to_text()).unwrap_or_else(|e| e);
		let shown = p.text("::Tactical.State != null ? \"results visible \" + ::Tactical.State.m.TacticalCombatResultScreen.isVisible() : \"\"").unwrap_or_else(|e| e);
		eprintln!("DIAG after a fresh animation: {after}; {shown}");
		let log: Vec<String> = p.session.log().into_iter().rev().take(25).map(|l| format!("  [{}] {}", l.level, l.text)).collect();
		eprintln!("DIAG {sq}\nDIAG page: {js}\nDIAG page log (newest first):\n{}", log.join("\n"));
		let shot = std::env::temp_dir().join("play_session_ui-failure.png");
		let saved = match p.session.screenshot(&shot) {
			Ok(Some(_)) => format!("\nlast frame: {}", shot.display()),
			_ => String::new(),
		};
		let states = p.session.engine().root_state().state_names();
		format!("{error}\nscript states: {states:?}{saved}")
	})
}

fn main() -> ExitCode {
	if let Some(code) = bb_ui::host::run_subprocess() {
		return ExitCode::from(code.clamp(0, 255) as u8);
	}
	let name = "a_long_play_session";
	let data_dir = Path::new(STEAM_DATA_DIR);
	if !data_dir.exists() {
		println!("test {name} ... skipped ({} not present)", data_dir.display());
		return ExitCode::SUCCESS;
	}
	match run(data_dir) {
		Ok(()) => {
			println!("test {name} ... ok");
			ExitCode::SUCCESS
		}
		Err(error) => {
			println!("test {name} ... FAILED\n{error}");
			ExitCode::FAILURE
		}
	}
}
