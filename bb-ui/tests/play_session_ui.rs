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
use std::time::{Duration, Instant};

use bb_engine::harness::STEAM_DATA_DIR;
use bb_engine::tactical_player as player;
use bb_ui::host::{Echo, HostConfig, MouseButton, UiSession};

#[path = "support/gpu_page.rs"]
mod gpu_page;

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
	/// The last event answer and how many times running it was given.
	last_answer: std::cell::RefCell<(String, u32)>,
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
		let deadline = self.deadline(timeout)?;
		loop {
			if self.int(&format!("({cond}) ? 1 : 0"))? == 1 {
				return Ok(());
			}
			if self.past(&deadline)? {
				return Err(format!("still not true: {cond}"));
			}
			self.session.step(1000.0 / 60.0)?;
		}
	}

	/// The game's own clock (s): its frames' time (`Time.getRealTimeF`, a 60th of a
	/// second a step here), which a busy machine doesn't run short as it does the
	/// clock's.
	fn game_time(&self) -> Result<f64, String> {
		Ok(self.int("(::Time.getRealTimeF() * 1000).tointeger()")? as f64 / 1000.0)
	}

	/// A limit `after` from now in the game's time, and as long on the clock: steps
	/// aren't paced, but the page's animations run on the clock and the scripts wait
	/// for some (a battle's turn bar, a loading screen's fade), so the game's time
	/// alone can run out before the page has had its time (a battle "took too long",
	/// more often with the bridge's calls cheap). The clock alone stops a loop that no
	/// longer advances the game, at ten times as long.
	fn deadline(&self, after: Duration) -> Result<Deadline, String> {
		let now = Instant::now();
		Ok(Deadline {
			game: self.game_time()? + after.as_secs_f64(),
			page: now + after,
			clock: now + after * 10,
		})
	}

	fn past(&self, deadline: &Deadline) -> Result<bool, String> {
		let now = Instant::now();
		Ok(self.game_time()? > deadline.game && now > deadline.page || now > deadline.clock)
	}
}

/// See [`Play::deadline`].
struct Deadline {
	game: f64,
	page: Instant,
	clock: Instant,
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
	pin_town_seeds(p)
}

/// Pin every town's shop and roster seeds, as the parity probes do: a town fills its
/// stock and recruits from them when first entered, and they hold the real time at its
/// creation (settlement.create), so prices and recruits, and the crowns left after
/// buying and hiring, would differ from run to run.
fn pin_town_seeds(p: &Play) -> Result<(), String> {
	p.script(
		r#"
		foreach (i, s in ::World.EntityManager.getSettlements())
		{
			s.m.ShopSeed = 1000 + i;
			s.m.RosterSeed = 2000 + i;
		}
		"#,
	)
}

/// Walk the company to the nearest town (with `building`, if given) and into it,
/// answering what comes up on the way. A town it's in already is left first.
fn enter_town(p: &Play, building: Option<&str>) -> Result<String, String> {
	match building {
		Some(b) => enter_town_where(p, &format!("s.hasBuilding(\"{b}\")")),
		None => enter_town_where(p, "true"),
	}
}

/// Walk the company to the nearest town for which the script condition `cond` (over
/// `s`, the settlement) holds, and into it, answering what comes up on the way. The
/// town it's in stays if it qualifies, else it's left first.
fn enter_town_where(p: &Play, cond: &str) -> Result<String, String> {
	let here = format!("::World.State.m.WorldTownScreen.isVisible() && (function(s) {{ return {cond}; }})(::World.State.getCurrentTown()) ? 1 : 0");
	if p.int(&here)? == 1 {
		p.script("::__town <- ::World.State.getCurrentTown();")?;
		return p.text("::World.State.getCurrentTown().getName()");
	}
	leave_town(p)?;
	p.script(&format!(
		r#"
		local player = ::World.State.getPlayer(), best = null, d = 0;
		foreach (s in ::World.EntityManager.getSettlements())
		{{
			if (!s.isEnterable() || !({cond})) continue;
			local dist = s.getTile().getDistanceTo(player.getTile());
			if (best == null || dist < d) {{ best = s; d = dist; }}
		}}
		if (best == null) throw "no town for the session's need";
		::__town <- best;
		"#
	))?;
	let town = p.text("::__town.getName()")?;
	p.script(WALK_TO)?;
	// Once on its way, the company keeps its path (finding one is a search).
	let go = r#"
		::World.State.m.AutoEnterLocation = ::WeakTableRef(::__town);
		if (!::World.State.getPlayer().hasPath()) ::__walkTo(::__town);
		::World.State.setPause(false);
	"#;
	let mut deadline = p.deadline(STEP * 3)?;
	let (set_out, clock) = (p.game_time()?, Instant::now());
	let mut last = String::new();
	let mut fought = 0;
	loop {
		if p.past(&deadline)? {
			let at = p.text(TRAVEL).unwrap_or_else(|e| e);
			return Err(format!("still travelling to {town} (facing {last}): {at}"));
		}
		if p.int("::World.State.m.LastEnteredTown != null && ::World.State.m.WorldTownScreen.isVisible() && ::World.State.getCurrentTown().getID() == ::__town.getID() ? 1 : 0")? == 1 {
			break;
		}
		let facing = p.text(FACING)?;
		// Entered, the town screen on its way: head nowhere (a destination set now would
		// take the company straight back in when it leaves).
		let entered = p.int("::World.State.m.LastEnteredTown != null ? 1 : 0")? == 1;
		match facing.as_str() {
			"map" | "done" if !entered => {
				p.script(go)?;
				p.session.step(1000.0 / 60.0)?;
			}
			"event" => answer_event(p)?,
			// A party on the road attacked the company, or drew it into its fight: fight
			// it out, then go on, the road's time anew (a battle's frames are the game's
			// time too, often more than the walk's).
			"combat" | "battle" | "results" => {
				answer(p, &facing, &last, &mut fought)?;
				if facing == "results" {
					deadline = p.deadline(STEP * 3)?;
				}
			}
			"town" => {
				// A town on the way (or the one just left, again): leave it.
				let here = p.text("::World.State.getCurrentTown() == null ? \"(none)\" : ::World.State.getCurrentTown().getName()")?;
				p.shot(&format!("passing-{here}"));
				during(&format!("leaving {here} on the way to {town}"), p.session.click_when_ready("Leave", Duration::from_secs(20)))?;
				p.until("!::World.State.m.WorldTownScreen.isVisible()", STEP)?;
			}
			_ => wait_frame(p)?,
		}
		last = facing;
	}
	println!("reached {town} in {:.0} s of the game's time ({:.0} s on the clock)", p.game_time()? - set_out, clock.elapsed().as_secs_f64());
	p.session.pump_until(Duration::from_secs(5), |s| !s.animating().unwrap_or(true));
	p.shot("town");
	Ok(town)
}

/// `::__walkTo(e)`: head the company for the world entity `e` as a player's click on it
/// does (world_state's onMouseInput): straight there when it's near, else along the
/// navigator's path. A destination far off alone walks a straight line, which stops
/// for good at the first mountain or sea in the way (the entity's `move` won't enter
/// impassable or ocean tiles, as in the original) and leaves the company standing
/// while the days, and its food, run out.
const WALK_TO: &str = r#"
	::__walkTo <- function(e) {
		local player = ::World.State.getPlayer(), dest = e.getPos();
		if (::getVecDistance(dest, player.getPos()) > ::Const.World.MovementSettings.PlayerDirectMoveRadius)
		{
			local nav = ::World.getNavigator().createSettings();
			nav.ActionPointCosts = ::Const.World.TerrainTypeNavCost;
			nav.RoadMult = 1.0 / ::Const.World.MovementSettings.RoadMult;
			local path = ::World.getNavigator().findPath(player.getTile(), e.getTile(), nav, 0);
			if (!path.isEmpty())
			{
				player.setPath(path);
				return;
			}
		}
		player.setPath(null);
		player.setDestination(dest);
	};
"#;

/// Where the company is on its way to `::__town`: one line.
const TRAVEL: &str = r#"(function() {
	local w = ::World.State, pl = w.getPlayer(), t = ::__town, pp = pl.getPos(), tp = t.getPos();
	return "at " + pp.X.tointeger() + "," + pp.Y.tointeger() + " (" + t.getName() + " at " + tp.X.tointeger() + "," + tp.Y.tointeger()
		+ ", " + pl.getTile().getDistanceTo(t.getTile()) + " tiles), destination " + (pl.m.Destination == null ? "none" : pl.m.Destination.X.tointeger() + "," + pl.m.Destination.Y.tointeger())
		+ ", path " + pl.hasPath() + ", paused " + w.isPaused() + ", auto-enter " + (w.m.AutoEnterLocation == null || w.m.AutoEnterLocation.isNull() ? "none" : w.m.AutoEnterLocation.getName())
		+ ", entered " + (w.m.LastEnteredTown == null ? "none" : w.getCurrentTown().getName()) + ", backsteps " + w.m.MenuStack.hasBacksteps()
		+ ", day " + ::World.getTime().Days + " hour " + ::World.getTime().Hours + ", food " + ::World.Assets.getFood() + ", brothers " + ::World.getPlayerRoster().getSize();
})()"#;

/// The centre of the `n`th element matching the CSS `css` that the player can see.
fn nth(p: &Play, css: &str, n: usize) -> Result<(i32, i32), String> {
	let found = p.session.eval(&format!(
		r#"(function () {{
			var seen = [];
			document.querySelectorAll({css:?}).forEach(function (el) {{
				if (el.offsetParent === null) return;
				var r = el.getBoundingClientRect();
				if (r.width > 0 && r.height > 0) seen.push([Math.round(r.left + r.width / 2), Math.round(r.top + r.height / 2)]);
			}});
			return seen.length > {n} ? seen[{n}] : null;
		}})()"#
	))?;
	let v = found.as_array().filter(|a| a.len() == 2).ok_or_else(|| format!("no element {n} of {css:?} on the page"))?;
	Ok((v[0].as_i64().unwrap_or(0) as i32, v[1].as_i64().unwrap_or(0) as i32))
}

/// Wait for the crowns to move from `before`, and check they moved by `-cost`.
fn paid(p: &Play, before: i64, cost: i64, what: &str) -> Result<(), String> {
	let money = || p.int("::World.Assets.getMoney()");
	let moved = p.session.pump_until(Duration::from_secs(10), |_| money().is_ok_and(|m| m != before));
	ensure(moved, format!("{what}: the crowns didn't move"))?;
	ensure(money()? == before - cost, format!("{what}: {before} - {cost} != {}", money()?))?;
	println!("{what}: {cost} crowns");
	Ok(())
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

/// Whether the page shows the item `id` (its instance id) in `owner`'s list ("shop" or
/// "stash"): the list is redrawn with it.
fn shows_item(p: &Play, owner: &str, id: &str) -> bool {
	let found = p.session.eval(&format!(
		r#"(function () {{
			var owner = 'world-town-screen-shop-dialog-module.{owner}', hit = false;
			jQuery('.ui-control').each(function () {{
				var d = jQuery(this).data('item');
				if (d && d.owner === owner && d.isEmpty === false && d.id === {id:?} && this.offsetParent !== null) hit = true;
			}});
			return hit;
		}})()"#
	));
	matches!(found, Ok(bb_ui::json::Json::Bool(true)))
}

/// Run the game until the page has stopped animating and `ready` holds.
fn settled(p: &Play, ready: impl Fn(&Play) -> bool) -> Result<(), String> {
	let ok = p.session.pump_until(STEP, |s| !s.animating().unwrap_or(true) && ready(p));
	ok.then_some(()).ok_or_else(|| "the page didn't settle".to_string())
}

/// Right-click at `(x, y)`, as a player buys or sells with (the callers wait for the
/// crowns to move).
fn right_click(p: &Play, x: i32, y: i32) -> Result<(), String> {
	p.session.move_to(x, y)?;
	p.session.host().mouse_button(x, y, MouseButton::Right, true);
	p.session.host().mouse_button(x, y, MouseButton::Right, false);
	Ok(())
}

/// Open the town's building `id` by clicking its picture (`slotN`, N its place in the
/// town's buildings, empty places counted).
fn open_building(p: &Play, id: &str) -> Result<(), String> {
	// Most buildings close at night: wait for the morning outside, as a player does.
	let closed = format!(
		"!::World.getTime().IsDaytime && ::World.State.getCurrentTown().getBuilding(\"{id}\").isClosedAtNight() ? 1 : 0"
	);
	if p.int(&closed)? == 1 {
		let town = p.int("::World.State.getCurrentTown().getID()")?;
		println!("{id} is closed at night: waiting for the morning");
		leave_town(p)?;
		p.script("::World.State.setPause(false); ::World.setSpeedMult(4.0);")?;
		during("waiting for the morning", out_on_the_map_until(p, "::World.getTime().IsDaytime", STEP * 3))?;
		p.script("::World.setSpeedMult(1.0);")?;
		enter_town_where(p, &format!("s.getID() == {town}"))?;
	}
	let slot = p.int(&format!(
		"(function() {{ foreach (i, b in ::World.State.getCurrentTown().m.Buildings) if (b != null && b.getID() == \"{id}\") return i; return -1; }})()"
	))?;
	ensure(slot >= 0, format!("no {id} in the town"))?;
	during(&format!("clicking the {id}"), p.session.click_when_ready(&format!("css:img.slot{slot}:not(.no-pointer-events)"), STEP))
}

/// A shop (the marketplace, a weaponsmith, an armourer…): buy the first item on sale,
/// then sell the first item of the company's, each with a right-click; the crowns move
/// by the item's price.
fn shop(p: &Play, building: &str) -> Result<(), String> {
	enter_town(p, Some(building))?;
	open_building(p, building)?;
	during("waiting for the shop's goods", settled(p, |p| item_at(p, "shop").is_ok()))?;
	p.shot(building);
	let money = || p.int("::World.Assets.getMoney()");
	let (x, y, idx) = item_at(p, "shop")?;
	let price = p.int(&format!("::World.State.getTownScreen().getShopDialogModule().getShop().getStash().getItemAtIndex({idx}).item.getBuyPrice()"))?;
	let name = p.text(&format!("::World.State.getTownScreen().getShopDialogModule().getShop().getStash().getItemAtIndex({idx}).item.getName()"))?;
	let id = p.text(&format!("::World.State.getTownScreen().getShopDialogModule().getShop().getStash().getItemAtIndex({idx}).item.getInstanceID()"))?;
	let before = money()?;
	right_click(p, x, y)?;
	during(&format!("buying {name} for {price}"), p.session.pump_until(Duration::from_secs(10), |_| money().is_ok_and(|m| m != before)).then_some(()).ok_or("the crowns didn't move".to_string()))?;
	ensure(money()? == before - price, format!("bought {name}: {before} - {price} != {}", money()?))?;
	// The page's lists are redrawn with it (in the stash, gone from the shop) before the
	// stash's first item is looked for there.
	during(&format!("{name} moving to the stash"), settled(p, |p| shows_item(p, "stash", &id) && !shows_item(p, "shop", &id)))?;
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
	during("leaving the shop", p.session.click_when_ready("Leave", STEP))?;
	Ok(())
}

/// The tavern's result under its `n`th button (0 the rumour's, 1 the round's), a page
/// expression.
fn tavern_result(n: usize) -> String {
	format!("jQuery('.l-tavern-button:visible').eq({n}).closest('.row.is-bottom').find('.is-result')")
}

/// Whether the tavern shows something under its `n`th button (emptied before it's paid).
fn told(p: &Play, n: usize) -> bool {
	let text = p.session.eval(&format!("{}.text().trim().length", tavern_result(n)));
	text.ok().and_then(|n| n.as_i64()).unwrap_or(0) > 0
}

/// The tavern: pay for a rumour, then for a round; each costs what its building says,
/// and the page tells what came of it.
fn tavern(p: &Play) -> Result<(), String> {
	enter_town(p, Some("building.tavern"))?;
	open_building(p, "building.tavern")?;
	during("waiting for the tavern", settled(p, |p| nth(p, ".l-tavern-button .ui-control", 1).is_ok()))?;
	// Both results empty, so what the payments show is theirs.
	for n in 0..2 {
		p.session.eval(&format!("{}.empty().length", tavern_result(n)))?;
	}
	p.shot("tavern");
	let money = || p.int("::World.Assets.getMoney()");
	let tavern = "::World.State.getCurrentTown().getBuilding(\"building.tavern\")";
	let rumour = p.int(&format!("{tavern}.getRumorPrice()"))?;
	let before = money()?;
	let (x, y) = nth(p, ".l-tavern-button .ui-control", 0)?;
	p.session.click_at(x, y)?;
	paid(p, before, rumour, "a rumour")?;
	during("the rumour", settled(p, |p| told(p, 0)))?;
	p.shot("rumour");
	let round = p.int(&format!("{tavern}.getDrinkPrice()"))?;
	let before = money()?;
	let (x, y) = nth(p, ".l-tavern-button .ui-control", 1)?;
	p.session.click_at(x, y)?;
	paid(p, before, round, "a round")?;
	during("the round", settled(p, |p| told(p, 1)))?;
	p.shot("round");
	during("leaving the tavern", p.session.click_when_ready("Leave", STEP))?;
	Ok(())
}

/// Pick the first brother in a building's roster list, then an option for him (a
/// training, an injury's treatment): the row `row` (a page expression over `e`, the
/// brother's entry), which costs `price` (a script expression over `b`, the brother).
fn roster_option(p: &Play, building: &str, row: &str, price: &str, what: &str) -> Result<(), String> {
	open_building(p, building)?;
	during("waiting for the roster", settled(p, |p| nth(p, ".list-entry", 0).is_ok()))?;
	during("picking a brother", p.session.click_when_ready("css:.list-entry", STEP))?;
	during("waiting for the options", settled(p, |p| nth(p, ".is-injury-row .is-button .ui-control", 0).is_ok()))?;
	p.shot(building);
	let id = p.session.eval("jQuery('.list-entry.is-selected').data('entry').ID")?.as_i64().ok_or("no brother selected")?;
	let cost = p.int(&format!("(function() {{ local b = ::Tactical.getEntityByID({id}); return {price}; }})()"))?;
	let row = p.session.eval(&format!("(function (e) {{ return {row}; }})(jQuery('.list-entry.is-selected').data('entry'))"))?.as_i64().unwrap_or(-1);
	ensure(row >= 0, format!("{what}: no such option for the brother"))?;
	let before = p.int("::World.Assets.getMoney()")?;
	let (x, y) = nth(p, ".is-injury-row .is-button .ui-control", row as usize)?;
	p.session.click_at(x, y)?;
	paid(p, before, cost, what)?;
	p.shot(&format!("{building}-done"));
	during("leaving", p.session.click_when_ready("Leave", STEP))?;
	Ok(())
}

/// The training hall: the first brother's sparring fight (80 + 50 per level), after
/// which he's trained.
fn training(p: &Play) -> Result<(), String> {
	enter_town(p, Some("building.training_hall"))?;
	roster_option(p, "building.training_hall", "0", "::Math.round(80 + 50 * b.getLevel())", "a sparring fight")?;
	ensure(p.int("(function() { foreach (b in ::World.getPlayerRoster().getAll()) if (b.getSkills().hasSkill(\"effects.trained\")) return 1; return 0; })()")? == 1, "nobody is trained")
}

/// The temple: a brother with a cut arm has it treated for its price. He may have come
/// by other injuries on the road (events hurt brothers), listed with it, in the skills'
/// order (by ID within one order: a bruised leg comes first): treat the cut arm's row.
fn temple(p: &Play) -> Result<(), String> {
	enter_town(p, Some("building.temple"))?;
	p.script("::World.getPlayerRoster().getAll()[0].getSkills().add(::new(\"scripts/skills/injury/cut_arm_injury\"));")?;
	let row = "e.Injuries.map(function (i) { return i.id; }).indexOf('injury.cut_arm')";
	roster_option(p, "building.temple", row, "b.getSkills().getSkillByID(\"injury.cut_arm\").getPrice()", "treating a cut arm")?;
	ensure(p.int("::World.getPlayerRoster().getAll()[0].getSkills().getSkillByID(\"injury.cut_arm\").isTreated() ? 1 : 0")? == 1, "the cut arm isn't treated")
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
	// One that takes the company to another town and seldom comes to blows (a delivery,
	// an escort), in the nearest town offering one.
	p.script(
		r#"::__calmContract <- function(s) {
			foreach (i, c in s.getContracts())
				if (["contract.deliver_item", "contract.escort_envoy"].find(c.getType()) != null) return i;
			return -1;
		};
		::__calmTowns <- function() {
			local n = 0;
			foreach (s in ::World.EntityManager.getSettlements())
				if (s.isEnterable() && ::__calmContract(s) >= 0) n++;
			return n;
		};"#,
	)?;
	// The offer may be gone by the time the company gets there (contracts run out):
	// then the next town offering one. When no town offers one, the factions post new
	// work as the days go by: wait for it out on the map.
	let mut tries = 0;
	let (town, index) = loop {
		if p.int("::__calmTowns()")? == 0 {
			println!("no town offers a delivery or escort: waiting for one");
			leave_town(p)?;
			during("waiting for a delivery or escort", out_on_the_map_until(p, "::__calmTowns() > 0", STEP * 6))?;
		}
		let town = enter_town_where(p, "::__calmContract(s) >= 0")?;
		let index = p.int("::__calmContract(::World.State.getCurrentTown())")?;
		if index >= 0 {
			break (town, index);
		}
		tries += 1;
		ensure(tries < 5, "no delivery or escort to be had")?;
		println!("the offer in {town} has gone: on to the next town");
	};
	let money = || p.int("::World.Assets.getMoney()");
	let start = money()?;
	println!("a contract in {town}: {}", p.text(&format!("::World.State.getCurrentTown().getContracts()[{index}].getType()"))?);
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

/// Answer the event on screen with its first option, as a player in a hurry does; but
/// after the same answer three times running (another roll of the dice, another
/// round…), its last option, as a player who has had enough does.
fn answer_event(p: &Play) -> Result<(), String> {
	let event = p.text("::World.Events.hasActiveEvent() ? ::World.Events.m.ActiveEvent.getID() : \"(none)\"")?;
	let options = "[id^=\"button-\"] .ui-control";
	during(&format!("waiting for {event}'s options"), settled(p, |p| nth(p, options, 0).is_ok()))?;
	let texts = p.session.eval(&format!("(function () {{ return jQuery('{options}:visible').map(function () {{ return jQuery(this).text().trim(); }}).get(); }})()"))?;
	let texts: Vec<String> = texts.as_array().map(|a| a.iter().map(|t| t.to_text()).collect()).unwrap_or_default();
	let first = format!("{event} {}", texts.first().cloned().unwrap_or_default());
	let repeats = {
		let last = p.last_answer.borrow();
		if last.0 == first { last.1 } else { 0 }
	};
	let n = if repeats >= 3 && texts.len() > 1 { texts.len() - 1 } else { 0 };
	let option = texts.get(n).cloned().unwrap_or_default();
	let (x, y) = nth(p, options, n)?;
	during(&format!("answering {event} with {option}"), p.session.click_at(x, y))?;
	*p.last_answer.borrow_mut() = if n == 0 { (first, repeats + 1) } else { (String::new(), 0) };
	println!("event {event}: {option}");
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
	// Fading out (on the clock, while the steps run fast): what's under it isn't back
	// yet (a contract's screen over a town hides the town's dialogs until it's gone).
	if (ev != null && ev.isAnimating()) return "wait";
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
		::__walkTo(target);
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
			// The turn bar slides each turn's unit in on the clock and finishes on it
			// (turnsequencebar_module.js): frames run as fast as they go would let a unit
			// act and die (its slot removed) before its slide is done, whose end then
			// fails on the removed slot. While the page animates, the frames are paced,
			// as the original runs them.
			if p.session.animating()? { wait_frame(p)? } else { p.session.step(1000.0 / 60.0)? }
		}
		"results" => {
			p.script("::__fightResult <- ::Tactical.Entities.getCombatResult();")?;
			// The footer button goes on to the loot, where there is any (take it all),
			// and then leaves. Its text says which ("Continue", "Leave"), but only once
			// the result's data has come: press the button itself while the screen
			// shows, once per settled screen (a press during its fade-out leaves twice).
			let gone = "!((\"State\" in ::Tactical) && ::Tactical.State != null)";
			let showing = "(\"State\" in ::Tactical) && ::Tactical.State != null && (function(r) { return r.isVisible() && !r.isAnimating(); })(::Tactical.State.m.TacticalCombatResultScreen) ? 1 : 0";
			let button = "css:.tactical-combat-result-screen .l-leave-button .ui-control";
			for press in 0..3 {
				settled(p, |_| true)?;
				if p.int(showing)? == 0 {
					break;
				}
				if press > 0 && p.session.eval("jQuery('.loot-all-button:visible').length")?.as_i64() == Some(1) {
					during("looting all", p.session.click_when_ready("css:.loot-all-button", STEP))?;
					p.session.pump_until(Duration::from_secs(1), |_| false);
					p.shot("looted");
				}
				let text = || p.session.eval("jQuery('.tactical-combat-result-screen .l-leave-button .ui-control').text()").map(|t| t.to_text()).unwrap_or_default();
				let before = text();
				during("leaving the battle", p.session.click_when_ready(button, STEP))?;
				// The scripts answer the press over the next frames: until the screen goes
				// on to the loot (the button reads anew) or away.
				let deadline = p.deadline(STEP)?;
				while p.int(showing)? == 1 && text() == before {
					ensure(!p.past(&deadline)?, "the result screen didn't answer the press")?;
					wait_frame(p)?;
				}
			}
			p.until(gone, STEP)?;
		}
		"event" => answer_event(p)?,
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
		_ => wait_frame(p)?,
	}
	Ok(())
}

/// One frame while the page animates (a screen fading in or out): paced, a 60th of a
/// second on the clock, as the original runs. The page's animations run on the clock,
/// and the world runs on under a screen fading out (the event screen's menu-stack pop
/// unpauses it as the fade begins): unpaced frames would walk the company much further
/// meanwhile than a player's does (into a town before a contract's screen, held back by
/// the fade, could show its arrival).
fn wait_frame(p: &Play) -> Result<(), String> {
	p.session.run_frames(1.0 / 60.0)
}

/// Carry the contract out: follow the map's marker to its target and back, answering
/// what comes up (see [`answer`]). Ends when the contract is paid.
fn adventure(p: &Play) -> Result<(), String> {
	bb_engine::harness::install_autoplay(p.session.engine())?;
	// The fallen die as the player's (off the roster), as when a player plays the battle.
	p.script("::__autoplayFight = true; ::__autoplayDieAsPlayers = true;")?;
	p.script(WALK_TO)?;
	during("leaving the town", p.session.click_when_ready("Leave", STEP))?;
	let money = || p.int("::World.Assets.getMoney()");
	let active = || p.int("::World.Contracts.getActiveContract() != null ? 1 : 0");
	let reward = p.int("::World.Contracts.getActiveContract().getPayment().getOnCompletion()")?;
	let deadline = p.deadline(Duration::from_secs(900))?;
	let (mut battles, mut last, mut paid) = (0, String::new(), None);
	loop {
		ensure(!p.past(&deadline)?, format!("the contract took too long (facing {last})"))?;
		let facing = p.text(FACING)?;
		if facing == "done" {
			break;
		}
		// The contract may lie far off (the nearest offer gone, the next one across the
		// map), and a company that runs out of food deserts: provisions, as a player
		// buys them before a long march.
		if facing == "map" && p.int("::World.Assets.getFood()")? < 30 {
			p.script("for (local i = 0; i < 4; i++) ::World.Assets.getStash().add(::new(\"scripts/items/supplies/ground_grains_item\")); ::World.Assets.updateFood();")?;
			println!("provisions for the road: food {}", p.int("::World.Assets.getFood()")?);
		}
		// The payment comes with the screen that ends the contract (wages go out on the
		// road meanwhile, so it's measured across that answer).
		let before = (money()?, active()?);
		answer(p, &facing, &last, &mut battles)?;
		if facing == "event" && before.1 == 1 && active()? == 0 {
			paid = Some(money()? - before.0);
		}
		last = facing;
	}
	let paid = paid.ok_or("the contract ended without its last screen")?;
	ensure(paid == reward, format!("paid {paid}, the contract said {reward}"))?;
	println!("contract done after {battles} battle(s): paid {reward}");
	Ok(())
}

/// A battle: the company meets the nearest hostile party, engages through the combat
/// dialog, fights it out (the AI playing the company, the fallen dying as the player's),
/// and takes the loot; back on the map, the loading screen is gone and the world shows.
fn battle(p: &Play) -> Result<(), String> {
	p.script(
		r#"::__nearestFoe <- function() {
			local player = ::World.State.getPlayer(), target = null, best = 0;
			foreach (e in ::World.getAllEntitiesAtPos(player.getPos(), 100000.0))
			{
				if (!e.isParty() || e.getID() == player.getID() || e.isAlliedWithPlayer() || e.getTroops().len() == 0) continue;
				local d = e.getDistanceTo(player);
				if (target == null || d < best) { target = e; best = d; }
			}
			return target;
		};"#,
	)?;
	// Hostile parties take to the roads as the days go by.
	leave_town(p)?;
	during("waiting for a hostile party", out_on_the_map_until(p, "::__nearestFoe() != null", STEP * 3))?;
	p.script(
		r#"
		local player = ::World.State.getPlayer(), target = ::__nearestFoe();
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
	let deadline = p.deadline(Duration::from_secs(600))?;
	let (mut battles, mut last) = (0, String::new());
	// Waiting after the battle (its loading screen fading out): 30 s, by the game's
	// time and the clock's (see `Play::deadline`).
	let mut waiting: Option<Deadline> = None;
	loop {
		ensure(!p.past(&deadline)?, format!("the battle took too long (facing {last})"))?;
		let facing = p.text(FACING)?;
		if battles > 0 && facing == "wait" {
			if waiting.is_none() {
				waiting = Some(p.deadline(Duration::from_secs(30))?);
			}
			if let Some(waiting) = &waiting {
				ensure(!p.past(waiting)?, "after the battle, still waiting 30 s on")?;
			}
		} else {
			waiting = None;
		}
		if battles > 0 && (facing == "map" || facing == "done") {
			break;
		}
		answer(p, &facing, &last, &mut battles)?;
		last = facing;
	}
	let result = p.int("(\"__fightResult\" in getroottable()) ? ::__fightResult : -1")?;
	let roster = p.int("::World.getPlayerRoster().getSize()")?;
	during("back on the map", p.until("!::LoadingScreen.isVisible() && ::World.State.m.WorldScreen.isVisible()", STEP))?;
	p.shot("after the battle");
	println!("fought {foe}: stash {before} -> {} (result {result}), {roster} brothers left", stash()?);
	Ok(())
}

/// Run the world, out on the map, until `cond` holds, answering events meanwhile and
/// keeping the world running (an event pauses it).
fn out_on_the_map_until(p: &Play, cond: &str, timeout: Duration) -> Result<(), String> {
	let deadline = p.deadline(timeout)?;
	while p.int(&format!("({cond}) ? 1 : 0"))? == 0 {
		ensure(!p.past(&deadline)?, format!("still not true: {cond}"))?;
		if p.text(FACING)? == "event" {
			answer_event(p)?;
		} else {
			p.script("if (::World.State.isPaused()) ::World.State.setPause(false);")?;
			p.session.step(1000.0 / 60.0)?;
		}
	}
	Ok(())
}

/// Leave the town the company is in, if any, and wait until it's out. The town screen
/// may be away for a moment (a contract's or an event's screen over it): leave once
/// it's back.
fn leave_town(p: &Play) -> Result<(), String> {
	let deadline = p.deadline(STEP)?;
	loop {
		let state = p.text("(::World.State.m.WorldTownScreen.isVisible() ? \"shown\" : \"away\") + \" \" + (::World.State.m.LastEnteredTown == null ? \"out\" : \"in\")")?;
		match state.as_str() {
			"away out" => return Ok(()),
			"shown in" | "shown out" if !p.session.animating()? => {
				during("leaving the town", p.session.click_when_ready("Leave", STEP))?;
				p.session.pump_until(Duration::from_secs(2), |s| !s.animating().unwrap_or(true));
			}
			_ if p.text(FACING)? == "event" => answer_event(p)?,
			_ => p.session.step(1000.0 / 60.0)?,
		}
		ensure(!p.past(&deadline)?, format!("still in town ({state})"))?;
	}
}

/// The retinue: a seat opens at 'Recognized' renown (250), and the cook joins once
/// the stash holds eight kinds of provisions; open the campfire, pick the free seat,
/// hire him for his price.
fn retinue(p: &Play) -> Result<(), String> {
	leave_town(p)?;
	p.script(
		r#"
		foreach (f in ["bread", "dried_fish", "goat_cheese", "ground_grains", "smoked_ham", "dried_fruits", "beer", "mead"])
			::World.Assets.getStash().add(::new("scripts/items/supplies/" + f + "_item"));
		// The renown of a few contracts done.
		::World.Assets.addBusinessReputation(300);
		"#,
	)?;
	// The page enables "Hire" only when the crowns cover the price (the scripts check
	// it too): the shops' first goods can have cost most of them by now.
	let cost = p.int("::World.Retinue.getFollower(\"follower.cook\").getCost()")?;
	p.script(&format!("local m = ::World.Assets.getMoney(); if (m < {cost}) ::World.Assets.addMoney({cost} - m);"))?;
	during("opening the retinue", p.session.click_when_ready("css:img[src$=\"ui/icons/campfire.png\"]", STEP))?;
	let free = "img[src$=\"ui/campfire/free_slot.png\"]:not(.no-pointer-events)";
	during("waiting for the campfire", settled(p, |p| nth(p, free, 0).is_ok()))?;
	p.shot("campfire");
	let (x, y) = nth(p, free, 0)?;
	p.session.click_at(x, y)?;
	during("waiting for the followers", settled(p, |p| nth(p, ".list-entry", 0).is_ok()))?;
	p.shot("followers");
	let unlocked = p.int("::World.Retinue.getFollower(\"follower.cook\").isUnlocked() ? 1 : 0")?;
	ensure(unlocked == 1, "the cook isn't unlocked with eight kinds of provisions")?;
	during("picking the cook", p.session.click_when_ready("The Cook", STEP))?;
	// "Hire" hires the picked entry, and does nothing while disabled (crowns short of
	// the price, the follower locked): wait for the cook picked and the button enabled.
	let picked = "(function () {
		var e = jQuery('.list-entry.is-selected').data('entry'), b = jQuery('.l-hire-button .ui-control');
		return 'picked ' + (e ? e.ID : 'none') + ', hire ' + (b.length === 0 ? 'missing' : b.is('[disabled]') ? 'disabled' : 'enabled');
	})()";
	let state = || p.session.eval(picked).map(|v| v.as_str().map_or_else(|| v.to_text(), str::to_string)).unwrap_or_else(|e| e);
	let ready = p.session.pump_until(STEP, |s| !s.animating().unwrap_or(true) && state() == "picked follower.cook, hire enabled");
	ensure(ready, format!("the cook can't be hired: {}; crowns {}, his price {cost}", state(), p.int("::World.Assets.getMoney()")?))?;
	let before = p.int("::World.Assets.getMoney()")?;
	during("hiring the cook", p.session.click_when_ready("Hire", STEP))?;
	paid(p, before, cost, "the cook")?;
	ensure(p.int("::World.Retinue.hasFollower(\"follower.cook\") ? 1 : 0")? == 1, "no cook in the retinue")?;
	p.shot("cook");
	for _ in 0..2 {
		if p.session.shows_text("Leave")? {
			during("leaving the campfire", p.session.click_when_ready("Leave", STEP))?;
			p.session.pump_until(Duration::from_secs(2), |s| !s.animating().unwrap_or(true));
		}
	}
	Ok(())
}

/// Pick the campaign entry `label` in the save or load list (its `.ui-control.campaign`;
/// the list's scroll container is a control with the same text when it holds one
/// entry), clicking again until it shows as picked: the list is filled in again when
/// the saves' data comes.
fn pick_campaign(p: &Play, label: &str) -> Result<(), String> {
	let entry = format!(
		r#"(function () {{
			var hit = null;
			jQuery('.ui-control.campaign:visible').each(function () {{ if (hit === null && this.textContent.trim().indexOf({label:?}) === 0) hit = this; }});
			if (hit === null) return null;
			var r = hit.getBoundingClientRect();
			return [Math.round(r.left + r.width / 2), Math.round(r.top + r.height / 2), hit.classList.contains('is-selected') ? 1 : 0];
		}})()"#
	);
	let at = |p: &Play| p.session.eval(&entry).ok().and_then(|v| v.as_array().map(|a| a.iter().map(|n| n.as_i64().unwrap_or(0)).collect::<Vec<_>>()));
	for _ in 0..10 {
		during(&format!("finding {label}"), settled(p, |p| at(p).is_some()))?;
		let v = at(p).ok_or(format!("no {label} in the list"))?;
		if v[2] == 1 {
			return Ok(());
		}
		p.session.click_at(v[0] as i32, v[1] as i32)?;
		p.session.pump_until(Duration::from_millis(500), |p2| at(p).is_some_and(|v| v[2] == 1) || !p2.animating().unwrap_or(true) && false);
	}
	Err(format!("{label} won't be picked"))
}

/// What a loaded campaign must have as it was saved, in one line.
const CAMPAIGN: &str = r#"(function() {
	local t = ::World.getTime(), a = ::World.Assets, pos = ::World.State.getPlayer().getPos();
	local s = "day " + t.Days + " hour " + t.Hours + " money " + a.getMoney() + " food " + a.getFood() + " ammo " + a.getAmmo()
		+ " parts " + a.getArmorParts() + " meds " + a.getMedicine() + " renown " + a.getBusinessReputation()
		+ " pos " + pos.X.tointeger() + "," + pos.Y.tointeger();
	s += " | bros";
	foreach (b in ::World.getPlayerRoster().getAll())
		s += " " + b.getName() + "/" + b.getBackground().getID() + "/L" + b.getLevel() + "/xp" + b.getXP() + "/hp" + b.getHitpoints() + "/" + b.getMoodState();
	s += " | stash";
	foreach (it in a.getStash().getItems()) if (it != null) s += " " + it.getID();
	s += " | contract " + (::World.Contracts.getActiveContract() == null ? "-" : ::World.Contracts.getActiveContract().getName());
	s += " | ambition " + (::World.Ambitions.hasActiveAmbition() ? ::World.Ambitions.getActiveAmbition().getID() : "-");
	s += " | retinue";
	foreach (f in ::World.Retinue.m.Followers) if (::World.Retinue.hasFollower(f.getID())) s += " " + f.getID();
	return s;
})()"#;

/// The start of every save name a test makes.
const SAVE_PREFIX: &str = "bb_probe_";

/// Save the campaign from the in-game menu (Escape, "Save Campaign", a new save named
/// `SAVE_PREFIX` + `name`), then load it ("Load Campaign", the save, "Load"): the
/// campaign loaded is the one saved.
fn save_and_load(p: &Play, name: &str) -> Result<(), String> {
	leave_town(p)?;
	p.script("::World.State.setPause(true);")?;
	let saved = p.text(CAMPAIGN)?;
	player::press_key(p.session.engine(), 41)?;
	during("the menu", p.session.click_when_ready("Save Campaign", STEP))?;
	during("waiting for the saves", settled(p, |p| nth(p, ".ui-control.campaign", 0).is_ok()))?;
	p.shot("save");
	during("a new save", pick_campaign(p, "New Savegame"))?;
	during("saving", p.session.click_when_ready("Save", STEP))?;
	during("naming the save", p.session.fill_field(".enter-name-container input", name))?;
	// The field takes letters, digits and spaces only (its keydown filter): the prefix's
	// underscores go in through the field's own setter.
	let name = &format!("{SAVE_PREFIX}{name}");
	p.session.eval(&format!("jQuery('.enter-name-container input').setInputText({name:?})"))?;
	during("confirming the name", p.session.click_when_ready("Ok", STEP))?;
	let listed = format!("(function() {{ foreach (s in ::PersistenceManager.queryStorages()) if (s.getName() == \"{name}\") return 1; return 0; }})()");
	during("the save being written", p.session.pump_until(Duration::from_secs(20), |_| p.int(&listed).is_ok_and(|n| n == 1)).then_some(()).ok_or("no save named so".to_string()))?;
	p.shot("saved");
	// Back to the menu's first page, then load what was saved.
	p.session.pump_until(Duration::from_secs(2), |s| !s.animating().unwrap_or(true));
	if !p.session.shows_text("Load Campaign")? {
		player::press_key(p.session.engine(), 41)?;
	}
	during("the menu", p.session.click_when_ready("Load Campaign", STEP))?;
	during("waiting for the saves", settled(p, |p| nth(p, ".ui-control.campaign", 0).is_ok()))?;
	during("picking the save", pick_campaign(p, name))?;
	p.shot("load");
	during("loading", p.session.click_when_ready("Load", STEP))?;
	p.session.pump_until(Duration::from_secs(2), |_| false);
	if p.session.shows_text("Ok")? {
		during("confirming the load", p.session.click_when_ready("Ok", STEP))?;
	}
	during("the loaded campaign", p.until("(\"State\" in ::World) && ::World.State != null && !::LoadingScreen.isVisible() && ::World.State.m.WorldScreen.isVisible()", STEP))?;
	p.shot("loaded");
	let loaded = p.text(CAMPAIGN)?;
	ensure(loaded == saved, format!("the loaded campaign differs:\nsaved  {saved}\nloaded {loaded}"))?;
	println!("saved and loaded: {saved}");
	Ok(())
}

/// What the town screen is, as the scripts and the page see it: one line.
fn town_state(p: &Play) -> String {
	let sq = p
		.text(r#"(function() {
			local w = ::World.State, t = w.m.WorldTownScreen;
			return "sq town visible " + t.isVisible() + " animating " + t.isAnimating() + " entered "
				+ (w.m.LastEnteredTown == null ? "none" : w.getCurrentTown().getName()) + " backsteps " + w.m.MenuStack.hasBacksteps()
				+ " world screen " + w.m.WorldScreen.isVisible();
		})()"#)
		.unwrap_or_else(|e| e);
	let page = p
		.session
		.eval("(function () { var t = jQuery('.world-town-screen'); return 'page town display ' + t.css('display') + ' opacity ' + t.css('opacity') + ', velocity ticking ' + (jQuery.Velocity && jQuery.Velocity.State.isTicking) + ' calls ' + (jQuery.Velocity ? jQuery.Velocity.State.calls.filter(function (c) { return !!c; }).length : -1) + ', buttons ' + jQuery('.ui-control:visible').length; })()")
		.map(|v| v.to_text())
		.unwrap_or_else(|e| e);
	format!("{sq}; {page}")
}

/// Print the town screen's state each frame for `frames` frames.
fn trace_town(p: &Play, what: &str, frames: u32) -> Result<(), String> {
	for f in 0..frames {
		println!("[{what} +{f}] {}", town_state(p));
		p.session.step(1000.0 / 60.0)?;
	}
	Ok(())
}

/// Enter a town, leave it, and on the next frame head straight back in, during the
/// leave's fade. The town ends entered with its screen hidden: vanilla's own race (the
/// page's show() only shortens the fade-out, whose completion then hides the screen
/// again), which the original shows too (probed).
/// The session waits for the fade before going back in, as a player does.
fn town_reentry(p: &Play) -> Result<(), String> {
	let town = enter_town(p, None)?;
	println!("[reentry] in {town}: {}", town_state(p));
	during("leaving the town", p.session.click_when_ready("Leave", STEP))?;
	trace_town(p, "left", 3)?;
	p.script("local pl = ::World.State.getPlayer(); ::World.State.m.AutoEnterLocation = ::WeakTableRef(::__town); pl.setDestination(::__town.getPos()); ::World.State.setPause(false);")?;
	trace_town(p, "back", 120)?;
	p.shot("reentered");
	during("the town again", p.session.click_when_ready("Leave", Duration::from_secs(20)))
}

/// Enter a town, open the tavern and leave it, then leave the town, tracing the town
/// screen frame by frame: the town screen should go and stay gone.
fn town_leave(p: &Play) -> Result<(), String> {
	enter_town(p, Some("building.tavern"))?;
	open_building(p, "building.tavern")?;
	during("waiting for the tavern", settled(p, |p| nth(p, ".l-tavern-button .ui-control", 1).is_ok()))?;
	during("leaving the tavern", p.session.click_when_ready("Leave", STEP))?;
	trace_town(p, "tavern left", 30)?;
	during("leaving the town", p.session.click_when_ready("Leave", STEP))?;
	trace_town(p, "town left", 120)?;
	ensure(p.int("::World.State.m.WorldTownScreen.isVisible() || ::World.State.m.LastEnteredTown != null ? 1 : 0")? == 0, "still in town")
}

fn play(p: &Play) -> Result<(), String> {
	match std::env::var("PLAY_ONLY").as_deref() {
		Ok("town_reentry") => {
			new_campaign(p)?;
			return town_reentry(p);
		}
		Ok("town_leave") => {
			new_campaign(p)?;
			return town_leave(p);
		}
		Ok("battle") => {
			new_campaign(p)?;
			bb_engine::harness::install_autoplay(p.session.engine())?;
			p.script("::__autoplayFight = true; ::__autoplayDieAsPlayers = true;")?;
			return battle(p);
		}
		Ok("retinue") => {
			new_campaign(p)?;
			retinue(p)?;
			return save_and_load(p, "play session");
		}
		_ => {}
	}
	new_campaign(p)?;
	// Few battles before the contract: nobody attacks the company. A party fighting
	// another next to it still draws it in (ai_world_attack), more often on the roads
	// it walks: the AI fights such a battle for it, the fallen dying as the player's.
	p.script("local pl = ::World.State.getPlayer(); pl.m.IsAttackable = false; pl.m.IsAttackableByAI = false;")?;
	bb_engine::harness::install_autoplay(p.session.engine())?;
	p.script("::__autoplayFight = true; ::__autoplayDieAsPlayers = true;")?;
	// A richer company than a new one, so that every purchase and hire can happen.
	p.script("::World.Assets.addMoney(10000);")?;
	let town = enter_town(p, None)?;
	println!("entered {town}");
	shop(p, "building.marketplace")?;
	hire(p)?;
	tavern(p)?;
	training(p)?;
	temple(p)?;
	for smith in ["building.weaponsmith", "building.armorsmith"] {
		shop(p, smith)?;
	}
	retinue(p)?;
	save_and_load(p, "play session")?;
	contract(p)?;
	adventure(p)?;
	// Fair game again: a battle with the nearest hostile party.
	p.script("local pl = ::World.State.getPlayer(); pl.m.IsAttackable = true; pl.m.IsAttackableByAI = true;")?;
	battle(p)?;
	// Each with the page's lines before it: what it was doing.
	let log = p.session.log();
	let problems: Vec<String> = log
		.iter()
		.enumerate()
		.filter(|(_, line)| line.level == "error" || line.level == "bridge")
		.map(|(i, line)| {
			let before: Vec<String> = log[i.saturating_sub(6)..i].iter().map(|l| format!("    after [{}] {}", l.level, l.text)).collect();
			format!("[{}] {}\n{}", line.level, line.text, before.join("\n"))
		})
		.collect();
	let stacks = p.session.eval("(window.__errorStacks || []).join('\\n---\\n')").map(|v| v.to_text()).unwrap_or_default();
	ensure(problems.is_empty(), format!("page or bridge errors:\n{}\nstacks:\n{stacks}", problems.join("\n")))
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
	let session = UiSession::start(data_dir, HostConfig { sink: Some(gpu_page::sink()?), ..HostConfig::default() }, Echo::Off)?;
	during("booting the menu", session.boot_menu())?;
	// The page's uncaught errors' stacks (its log has only their messages).
	session.eval("window.__errorStacks = []; window.addEventListener('error', function (e) { window.__errorStacks.push(e.error && e.error.stack ? e.error.stack : String(e.message)); }); 0")?;
	let shots = std::env::var_os("PLAY_SHOTS").map(PathBuf::from);
	if let Some(dir) = &shots {
		let _ = std::fs::create_dir_all(dir);
	}
	let p = Play { session, shots, step: std::cell::Cell::new(0), last_answer: Default::default() };
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
	// Chromium paints the page through a display connection: a headless gamescope's.
	bb_ui::host::with_display(run_main)
}

fn run_main() -> ExitCode {
	let name = "a_long_play_session";
	let data_dir = Path::new(STEAM_DATA_DIR);
	if !data_dir.exists() {
		return bb_engine::harness::skip_case(name, format!("{} not present", data_dir.display()));
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
