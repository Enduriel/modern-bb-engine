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

fn play(p: &Play) -> Result<(), String> {
	new_campaign(p)?;
	let town = enter_town(p)?;
	println!("entered {town}");
	market(p)?;
	hire(p)?;
	let problems: Vec<String> = p
		.session
		.log()
		.into_iter()
		.filter(|line| line.level == "error" || line.level == "bridge")
		.map(|line| format!("[{}] {}", line.level, line.text))
		.collect();
	ensure(problems.is_empty(), format!("page or bridge errors:\n{}", problems.join("\n")))
}

fn run(data_dir: &Path) -> Result<(), String> {
	let session = UiSession::start(data_dir, HostConfig::default(), false)?;
	during("booting the menu", session.boot_menu())?;
	let shots = std::env::var_os("PLAY_SHOTS").map(PathBuf::from);
	if let Some(dir) = &shots {
		let _ = std::fs::create_dir_all(dir);
	}
	let p = Play { session, shots, step: std::cell::Cell::new(0) };
	play(&p).map_err(|error| {
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
