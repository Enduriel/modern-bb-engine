//! Where the game's memory goes: the game as `bb` plays it, offscreen, through a
//! session, measured by owner at fixed points.
//!
//! ```text
//! cargo run --release -p bb-app --features ui --example memory_inventory -- \
//!     [--battles N] [--every-battle] [--days N] [--until POINT] [--seed SEED] [--mods]
//!     [--out REPORT.md] [--devtools PORT --hold SECONDS]
//! cargo run --release -p bb-app --example memory_inventory -- --engine-only [--mods]
//! ```
//!
//! The session: a new campaign's world; `--battles` battles (default 12), each against a
//! party of another people or beast (made by vanilla's spawn lists, next to the company),
//! played by the AI on both sides until it ends (the company's AI often retreats; the
//! company is made hardy and kept at three brothers); `--days` days of the world running (default 10; the parties leave the company be, and a battle
//! that comes up anyway is fought out); quitting to the main menu; and a second
//! campaign (`--until` ends it after the point whose name starts so, e.g. "after battle
//! 1"). A battle the AI can't end in 20 minutes of the game's time, the company flees.
//! It measures after the first battle, the middle one and the last (after every one with
//! `--every-battle`), and the other points. `--mods` plays it with the install's mods
//! (their storages in memory; MSU's new-campaign settings saved as they are). At each
//! point it measures:
//! - every process of the game's (with `--features ui`, CEF's too): PSS by the kind of
//!   memory holding it, and the GPU memory it allocated (`bb_app::stats::tree_memory`);
//! - the C heap: what is in use, and what malloc holds free;
//! - the renderer's textures and pages and what the frames drew of them since the point
//!   before, and wgpu's allocations and the blocks it reserves, and what the game
//!   process's GPU memory holds beyond them (the driver's own, and Chromium's GPU work,
//!   which runs in this process: on amdgpu its GL context shares our device, so its
//!   memory counts in our DRM client);
//! - the engine's records: tactical units, world entities, saves kept in memory, the
//!   sound bank (on a silent output: nothing is played on the device);
//! - with the UI, the page's log.
//!
//! With `--devtools`, the page serves DevTools on that port, and `--hold` keeps the game
//! running at its real pace that long after the last point, for Chromium's own memory
//! dump (memory-infra over the DevTools protocol).
//!
//! `--engine-only` measures the engine alone, headless (no renderer, no page), at the main
//! menu and on a new campaign's world; with `--mods`, with the install's mods mounted (bb
//! doesn't draw mods' sprites yet, so this is how mods' share is measured).
//!
//! The report is Markdown, written to `--out` (or printed). Linux only for now (the
//! processes' breakdown).

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::rc::Rc;
use std::time::{Duration, Instant};

use bb_app::game::Game;
use bb_app::stats::{self, MappingKind, ProcessMemory};
use bb_audio::{KiraOut, MockBackend};
use bb_engine::files::GameFiles;
use bb_engine::harness::{self, STEAM_DATA_DIR};
use bb_engine::tactical_player as player;
use bb_render::{Gpu, RenderTarget};
use squirrel_bb::{AudioOut, VideoMode, Voice};

/// The real time a frame advances the game by: settling and in battle (a frame's real
/// time, as the windowed game's; longer frames can stall a battle's turns), and while
/// days pass on the world map.
const SETTLE_MS: f64 = 1000.0 / 60.0;
const BATTLE_MS: f64 = SETTLE_MS;
const DAYS_MS: f64 = 250.0;

/// Draw one frame in this many while time runs fast (every frame while settling).
const DRAW_EVERY: u32 = 8;

/// The least time on the clock a wait for the page gets ([`Session::until_page`]).
const PAGE_TIME: Duration = Duration::from_secs(30);

const IN_TACTICAL: &str = "(\"State\" in ::Tactical) && ::Tactical.State != null && !::LoadingScreen.isVisible()";
const BACK_IN_WORLD: &str = "(!(\"State\" in ::Tactical) || ::Tactical.State == null) && ::World.isVisible() && !::LoadingScreen.isVisible() && (\"__lastCombatResult\" in getroottable())";
const AT_MENU: &str = "::RootState.get(\"WorldState\") == null && ::RootState.get(\"MainMenuState\").m.MainMenuScreen.isVisible() && !::RootState.get(\"MainMenuState\").m.MainMenuScreen.isAnimating()";
const IN_WORLD: &str = "::RootState.get(\"WorldState\") != null && (\"State\" in ::World) && ::World.State != null && ::World.State.getPlayer() != null && !::LoadingScreen.isVisible()";

/// The `n`th battle's opponent: a party vanilla's own spawn lists make, of the factions in
/// turn (brigands, beasts, the undead, greenskins, barbarians, nomads), next to the company,
/// so the battles draw other peoples and beasts (any hostile party left there from an
/// earlier battle is removed first); one that can't be made (a DLC's missing) falls back
/// to the nearest hostile party. The company is made hardy (hit points) and kept at three
/// brothers at least, recruited as the scenario's are.
const ENGAGE: &str = r#"
local player = ::World.State.getPlayer();
local kinds = [
	["Bandits", "Brigand Raiders", "BanditRaiders"], ["Beasts", "Direwolves", "Direwolves"],
	["Undead", "Undead", "UndeadArmy"], ["Orcs", "Orc Marauders", "OrcRaiders"],
	["Goblins", "Goblin Raiders", "GoblinRaiders"], ["Zombies", "Wiedergaenger", "Zombies"],
	["Barbarians", "Barbarians", "Barbarians"], ["Beasts", "Nachzehrers", "Ghouls"],
	["Beasts", "Webknechts", "Spiders"], ["OrientalBandits", "Nomads", "NomadRaiders"],
	["Beasts", "Unholds", "Unhold"], ["Beasts", "Schrats", "Schrats"]
];
local kind = kinds[({n} - 1) % kinds.len()];
local roster = ::World.getPlayerRoster();
if (roster.getSize() == 0) throw "the company is gone (wiped out in the last battle: the game is over)";
// Three brothers at least, recruited as the scenario's are (the AI loses some).
while (roster.getSize() < 3)
{
	local bro = roster.create("scripts/entity/tactical/player");
	bro.m.HireTime = ::Time.getVirtualTimeF();
	bro.setStartValuesEx(["companion_1h_background"]);
}
// A party left from an earlier battle at the company's place would join this one.
foreach (e in ::World.getAllEntitiesAtPos(player.getPos(), 200.0))
	if (e.isParty() && e.getID() != player.getID() && !e.isAlliedWithPlayer()) e.die();
local target = null;
try
{
	local faction = ::World.FactionManager.getFactionOfType(::Const.FactionType[kind[0]]);
	target = faction.spawnEntity(player.getTile(), kind[1], false, ::Const.World.Spawn[kind[2]], 120);
}
catch (e)
{
	::logInfo("memory_inventory: no " + kind[1] + " (" + e + "), the nearest hostile party instead");
}
if (target == null)
{
	local best = 0;
	foreach (e in ::World.getAllEntitiesAtPos(player.getPos(), 1000000.0))
	{
		if (!e.isParty() || e.getID() == player.getID() || e.isAlliedWithPlayer() || e.getTroops().len() == 0) continue;
		local d = e.getDistanceTo(player);
		if (target == null || d < best) { target = e; best = d; }
	}
	if (target == null) throw "no hostile party to fight";
	local tp = target.getPos();
	player.setPos(::createVec(tp.X - 30, tp.Y));
	::World.setPlayerPos(player.getPos());
}
::__inventoryTarget <- target.getName() + " (" + target.getTroops().len() + ", terrain " + target.getTile().Type + ")";
foreach (bro in ::World.getPlayerRoster().getAll())
{
	bro.getBaseProperties().Hitpoints = 900;
	bro.getSkills().update();
	bro.setHitpoints(bro.getHitpointsMax());
}
::World.getCamera().setPos(player.getPos());
::__autoplayEngage = true;
::__autoplayFight <- true;
// When every enemy flees, the battle asks whether it's over (and pauses until answered):
// "It's over", as a player would.
if (!("__inventoryAutoplay" in getroottable()))
{
	::__inventoryAutoplay <- ::__autoplay;
	::__autoplay = function()
	{
		if (("State" in ::Tactical) && ::Tactical.State != null)
		{
			local st = ::Tactical.State;
			local d = st.m.TacticalDialogScreen;
			if (st.m.IsEnemyRetreatDialogShown && d.isVisible() && !d.isAnimating())
			{
				st.tactical_retreat_screen_onYesPressed();
				return;
			}
		}
		::__inventoryAutoplay();
	};
}
// A brother who falls leaves the roster, as the player's do (the AI plays them).
::__autoplayDieAsPlayers <- true;
if ("__lastCombatResult" in getroottable()) delete ::__lastCombatResult;
::World.State.showCombatDialog(true, true, true, null, player.getPos());
"#;

/// Battle frames (at [`BATTLE_MS`]) a battle may take: 20 minutes of the game's time.
const BATTLE_FRAMES: u32 = 72_000;

/// Once the battle is under way and sampled: every enemy left at 1 hit point, so the
/// company wins it soon, its last blow ending it as battles end.
const WEAKEN: &str = r#"
foreach (faction in ::Tactical.Entities.getAllInstances())
	foreach (e in faction)
		if (e != null && e.isAlive() && !e.isDying())
		{
			if (!e.isAlliedWithPlayer()) e.setHitpoints(1);
			else
			{
				e.setHitpoints(e.getHitpointsMax());
				// A brother who flees and has nowhere to go skips his turns: rallied, he
				// fights (with the heals, a surrounded one fled forever once).
				if (e.getMoraleState() < ::Const.MoraleState.Steady) e.setMoraleState(::Const.MoraleState.Steady);
				// Bitten often enough, he's too fatigued for any skill or step.
				e.setFatigue(0);
			}
		}
"#;

/// Every unit on the map, for a battle that won't end.
const UNITS: &str = r#"(function() {
local s = "units:";
foreach (faction in ::Tactical.Entities.getAllInstances())
	foreach (e in faction)
		if (e != null && e.isAlive() && e.isPlacedOnMap())
		{
			local t = e.getTile();
			local adj = 0;
			for (local i = 0; i < 6; i++)
				if (t.hasNextTile(i) && t.getNextTile(i).IsOccupiedByActor && !t.getNextTile(i).getEntity().isAlliedWith(e)) adj++;
			local a = e.getAIAgent();
			s += " [" + e.getName() + " hp " + e.getHitpoints() + " morale " + e.getMoraleState() + " fatigue " + e.getFatigue() + "/" + e.getFatigueMax() + " AP " + e.getActionPoints()
				+ " pc " + e.isPlayerControlled() + " at " + t.X + "," + t.Y + " enemies adj " + adj
				+ (a == null ? "" : " agent " + a.ClassName + " finished " + a.isFinished()) + "]";
		}
return s;
})()"#;

/// Battle frames between two [`WEAKEN`]s: the AI misses, and the beasts' bites add up
/// over a long battle (direwolves wiped the company out once), and the company's morale
/// breaks and its fatigue fills.
const WEAKEN_EVERY: u32 = 600;

/// What a battle that won't end is waiting on.
const STALL: &str = r#"(function() {
	local s = "";
	try {
		local bar = ::Tactical.TurnSequenceBar;
		local a = bar.getActiveEntity();
		local st = ::Tactical.State;
		s += "round " + bar.getCurrentRound() + ", battle ended " + st.isBattleEnded();
		// After the end, the results screen comes from a real-time event (onBattleEnded ->
		// onBattleEndedDelayed), which waits while the menu stack has backsteps.
		s += ", menu backsteps " + st.m.MenuStack.hasBacksteps() + ", real-time events " + ::Time.hasEventScheduled(::TimeUnit.Real)
			+ ", results " + (st.m.TacticalCombatResultScreen != null && st.m.TacticalCombatResultScreen.isVisible());
		if (a == null) return s + ", no active entity";
		s += ", active " + a.getName() + " (" + a.getID() + ") turn started " + a.isTurnStarted() + ", done " + a.isTurnDone()
			+ ", placed " + a.isPlacedOnMap() + ", hp " + a.getHitpoints() + ", AP " + a.getActionPoints()
			+ ", player controlled " + a.isPlayerControlled() + ", hidden " + a.isHiddenToPlayer();
		local agent = a.getAIAgent();
		if (agent != null) s += ", agent " + agent.ClassName + " finished " + agent.isFinished()
			+ " evaluating " + agent.isEvaluating() + " behavior " + (agent.m.ActiveBehavior == null ? "null" : agent.m.ActiveBehavior.getID());
		s += "; game paused " + st.m.IsGamePaused + ", AI paused " + st.m.IsAIPaused + ", auto retreat " + st.m.IsAutoRetreat
			+ ", input locked " + st.m.IsInputLocked + ", camera delayed " + ::Tactical.CameraDirector.isDelayed()
			+ ", travelling " + ::Tactical.getNavigator().IsTravelling + ", action state " + st.m.CurrentActionState
			+ ", dialog " + ::DialogScreen.isVisible() + ", tactical dialog " + st.m.TacticalDialogScreen.isVisible()
			+ " (enemy retreat asked " + st.m.IsEnemyRetreatDialogShown + "), tactical menu " + st.m.TacticalMenuScreen.isVisible()
			+ ", character screen " + st.m.CharacterScreen.isVisible() + ", virtual speed " + ::Time.getVirtualSpeed();
	} catch (e) { s += " (" + e + ")"; }
	return s;
})()"#;

/// Why the world's clock might not run.
const WORLD_STATE: &str = r#"(function() {
	local w = ::World.State, t = ::World.getTime();
	local ev = w.getEventScreen();
	return "day " + t.Days + " (" + t.Time + "), paused " + w.isPaused() + ", speed " + ::World.getSpeedMult()
		+ ", event " + (ev != null && ev.isVisible()) + ", combat dialog " + (w.m.CombatDialog != null && w.m.CombatDialog.isVisible())
		+ ", menu stack " + (("MenuStack" in w.m) && w.m.MenuStack != null ? w.m.MenuStack.hasBacksteps() : "-")
		+ ", loading " + ::LoadingScreen.isVisible() + ", town " + w.m.WorldTownScreen.isVisible();
})()"#;

struct Options {
	data: PathBuf,
	seed: String,
	battles: u32,
	days: u32,
	until: Option<String>,
	out: Option<PathBuf>,
	devtools: Option<u16>,
	hold: u64,
	engine_only: bool,
	mods: bool,
	/// Sample after every battle, not only the first, the middle one and the last.
	every_battle: bool,
}

fn options() -> Result<Options, String> {
	let mut o = Options {
		data: PathBuf::from(STEAM_DATA_DIR),
		seed: "rust-engine-test".to_string(),
		battles: 12,
		days: 10,
		until: None,
		out: None,
		devtools: None,
		hold: 0,
		engine_only: false,
		mods: false,
		every_battle: false,
	};
	let mut args = std::env::args().skip(1);
	while let Some(arg) = args.next() {
		let mut value = || args.next().ok_or(format!("{arg} needs a value"));
		match arg.as_str() {
			"--data" => o.data = PathBuf::from(value()?),
			"--seed" => o.seed = value()?,
			"--battles" => o.battles = value()?.parse().map_err(|_| "--battles N")?,
			"--days" => o.days = value()?.parse().map_err(|_| "--days N")?,
			"--out" => o.out = Some(PathBuf::from(value()?)),
			"--until" => o.until = Some(value()?),
			"--devtools" => o.devtools = Some(value()?.parse().map_err(|_| "--devtools PORT")?),
			"--hold" => o.hold = value()?.parse().map_err(|_| "--hold SECONDS")?,
			"--engine-only" => o.engine_only = true,
			"--mods" => o.mods = true,
			"--every-battle" => o.every_battle = true,
			_ => return Err(format!("unknown argument {arg}")),
		}
	}
	Ok(o)
}

/// The game's sound on kira's mock backend, shared so the bank can be measured; it
/// renders as the frames' real time passes, so voices end as on a device.
struct SilentOut {
	out: Rc<RefCell<KiraOut<MockBackend>>>,
	last_ms: Option<u32>,
}

impl AudioOut for SilentOut {
	fn start(&mut self, file: &str, at: Option<(f32, f32)>) -> Option<Voice> {
		self.out.borrow_mut().start(file, at)
	}
	fn set_volume(&mut self, voice: Voice, volume: f32) {
		self.out.borrow_mut().set_volume(voice, volume)
	}
	fn set_pan(&mut self, voice: Voice, pan: f32) {
		self.out.borrow_mut().set_pan(voice, pan)
	}
	fn set_speed(&mut self, voice: Voice, speed: f32) {
		self.out.borrow_mut().set_speed(voice, speed)
	}
	fn set_paused(&mut self, voice: Voice, paused: bool) {
		self.out.borrow_mut().set_paused(voice, paused)
	}
	fn stop(&mut self, voice: Voice) {
		self.out.borrow_mut().stop(voice)
	}
	fn release(&mut self, voice: Voice) {
		self.out.borrow_mut().release(voice)
	}
	fn position_ms(&self, voice: Voice) -> u32 {
		self.out.borrow().position_ms(voice)
	}
	fn stopped(&self, voice: Voice) -> bool {
		self.out.borrow().stopped(voice)
	}
	fn length_ms(&mut self, file: &str) -> u32 {
		self.out.borrow_mut().length_ms(file)
	}
	fn preload(&mut self, file: &str) {
		self.out.borrow_mut().preload(file)
	}
	fn set_listener(&mut self, at: (f32, f32), half_width: f32) {
		self.out.borrow_mut().set_listener(at, half_width)
	}
	fn update(&mut self, now_ms: u32) {
		let mut out = self.out.borrow_mut();
		if let Some(last) = self.last_ms {
			// At most a second at once: a long frame renders no more than the device's
			// buffers would.
			out.run_silent(now_ms.saturating_sub(last).min(1000));
		}
		self.last_ms = Some(now_ms);
		out.update(now_ms);
	}
}

/// The memory at one point of the session.
struct Sample {
	point: String,
	processes: Vec<ProcessMemory>,
	/// malloc's: bytes in use, bytes it holds free.
	heap_used: u64,
	heap_free: u64,
	renderer: bb_render::RendererMemory,
	/// The atlases' parts drawn since the last point (`Game::take_texture_usage`).
	drawn: Vec<bb_render::AtlasUse>,
	/// wgpu's memory allocator: bytes its live allocations take, bytes its blocks reserve,
	/// and the allocations' bytes by name (a texture's or buffer's label).
	gpu_allocated: u64,
	gpu_reserved: u64,
	gpu_by_name: Vec<(String, u64)>,
	units: usize,
	dead_units: usize,
	world_entities: usize,
	saves: (usize, usize),
	/// The sound bank: sounds, decoded bytes, streamed bytes playing, bytes read ahead.
	sounds: (usize, usize, usize, usize),
	/// The sound output's voices: all, never started, ended but not released.
	voices: (usize, usize, usize),
	ui_log: Option<usize>,
}

/// Under jemalloc with profiling on (`LD_PRELOAD=libjemalloc.so.2 MALLOC_CONF=prof:true`)
/// and `BB_HEAP_PROFILE_DIR` set: dump a heap profile of this moment there, named after
/// the point (`parity/perf/heap_owners.py` reads it). Without them, nothing.
fn heap_profile(index: usize, point: &str) {
	let Some(dir) = std::env::var_os("BB_HEAP_PROFILE_DIR") else { return };
	let name: String = point.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
	let path = Path::new(&dir).join(format!("{index:02}-{name}.heap"));
	if let Err(e) = bb_app::heap_profile::dump(&path) {
		eprintln!("heap profile {}: {e}", path.display());
	}
}

struct Session<'a> {
	game: Game,
	gpu: &'a Gpu,
	target: RenderTarget,
	sound: Rc<RefCell<KiraOut<MockBackend>>>,
	samples: Vec<Sample>,
	battles: Vec<String>,
	until: Option<String>,
}

/// What [`Session::sample`] returns at the `--until` point, to end the session there.
const UNTIL: &str = "stopped at the --until point";

impl Session<'_> {
	fn engine(&self) -> &harness::GameEngine {
		self.game.front().engine()
	}

	/// One frame of `ms`, the scripted player's turn, and every `draw_every`th a drawn
	/// frame (atlases load as frames need them).
	fn frame(&mut self, ms: f64, n: u32, draw_every: u32) -> Result<(), String> {
		self.game.step(ms).map_err(|e| format!("a frame: {e}"))?;
		let autoplay = squirrel::SqTable::root(self.engine().vm()).get::<squirrel::SqStrongRef>("__autoplay");
		if let Some(f) = autoplay {
			// It can catch the tactical state half torn down between battles; it plays on
			// next frame.
			if let Err(e) = f.call(self.engine().vm(), ()) {
				eprintln!("autoplay: {e}");
			}
		}
		if n % draw_every == 0 {
			self.game.draw(self.gpu, &self.target);
			self.gpu.device.poll(wgpu::PollType::Poll).map_err(|e| e.to_string())?;
		}
		Ok(())
	}

	fn frames(&mut self, ms: f64, count: u32, draw_every: u32) -> Result<(), String> {
		(0..count).try_for_each(|n| self.frame(ms, n, draw_every))
	}

	fn holds(&self, cond: &str) -> Result<bool, String> {
		Ok(player::eval_i64(self.engine(), &format!("({cond}) ? 1 : 0"))? == Some(1))
	}

	fn until(&mut self, ms: f64, max: u32, what: &str, cond: &str) -> Result<(), String> {
		self.until_after(ms, max, Duration::ZERO, what, cond)
	}

	/// [`until`](Self::until) for what the page shows: also not given up before
	/// [`PAGE_TIME`] has passed on the clock. The frames aren't paced, but the page's
	/// animations run on the clock (the loading screen reports itself hidden when its
	/// fade ends), and with the bridge's calls cheap, 3000 frames can pass in a second.
	fn until_page(&mut self, ms: f64, max: u32, what: &str, cond: &str) -> Result<(), String> {
		self.until_after(ms, max, PAGE_TIME, what, cond)
	}

	/// [`until_page`](Self::until_page) for a new campaign's world, MSU's new-campaign
	/// settings saved as they are when its page shows (with `--mods`).
	fn until_world(&mut self, what: &str) -> Result<(), String> {
		const MSU_SETTINGS: &str = "(\"MSU\" in ::getroottable()) && ::MSU.SettingsScreen.isVisible()";
		let start = Instant::now();
		let mut n = 0;
		while !self.holds(IN_WORLD)? {
			if self.holds(MSU_SETTINGS)? {
				self.engine().run_script("::MSU.SettingsScreen.onSaveButtonPressed({});", "MSU settings")?;
			}
			if n >= 6000 && start.elapsed() >= PAGE_TIME {
				return Err(format!("{what}: not after {n} frames"));
			}
			self.frame(SETTLE_MS, n, DRAW_EVERY)?;
			n += 1;
		}
		Ok(())
	}

	fn until_after(&mut self, ms: f64, max: u32, clock: Duration, what: &str, cond: &str) -> Result<(), String> {
		let start = Instant::now();
		let mut n = 0;
		loop {
			if self.holds(cond)? {
				return Ok(());
			}
			if n >= max && start.elapsed() >= clock {
				return Err(format!("{what}: not after {n} frames ({cond})"));
			}
			self.frame(ms, n, DRAW_EVERY)?;
			n += 1;
		}
	}

	fn sample(&mut self, point: &str) -> Result<(), String> {
		// Let the renderer take in what its workers decoded, and the GPU catch up.
		let _ = self.frames(SETTLE_MS, 4, 1);
		heap_profile(self.samples.len(), point);
		let drawn = self.game.take_texture_usage().unwrap_or_default();
		let engine = self.engine();
		let m = bb_engine::heap::usage().unwrap_or_default();
		let (units, dead_units) = (engine.tactical_entities().unit_count(), engine.tactical_entities().dead_unit_count());
		let (gpu_allocated, gpu_reserved, gpu_by_name) = match self.gpu.device.generate_allocator_report() {
			Some(report) => {
				let mut by_name: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
				for a in &report.allocations {
					*by_name.entry(a.name.clone()).or_default() += a.size;
				}
				let mut by_name: Vec<(String, u64)> = by_name.into_iter().collect();
				by_name.sort_by(|a, b| b.1.cmp(&a.1));
				(report.total_allocated_bytes, report.total_reserved_bytes, by_name)
			}
			None => (0, 0, Vec::new()),
		};
		#[cfg(feature = "ui")]
		let ui_log = Some(self.game.front().session().log().len());
		#[cfg(not(feature = "ui"))]
		let ui_log = None;
		let sample = Sample {
			point: point.to_string(),
			processes: stats::tree_memory().unwrap_or_default(),
			heap_used: m.in_use as u64,
			heap_free: m.free as u64,
			renderer: self.game.renderer_memory(),
			drawn,
			gpu_allocated,
			gpu_reserved,
			gpu_by_name,
			units,
			dead_units,
			world_entities: engine.world_entities().count(),
			saves: engine.saves().memory_use(),
			sounds: self.sound.borrow().bank_use(),
			voices: self.sound.borrow().voice_use(),
			ui_log,
		};
		eprintln!(
			"{point}: {} MB PSS, {} MB GPU, renderer {}, units {units} ({dead_units} dead)",
			mb(sample.processes.iter().map(ProcessMemory::pss).sum()),
			mb(sample.processes.iter().map(|p| p.vram + p.gtt).sum()),
			sample.renderer
		);
		self.samples.push(sample);
		match &self.until {
			Some(until) if point.starts_with(until.as_str()) => Err(UNTIL.to_string()),
			_ => Ok(()),
		}
	}

	fn battle(&mut self, n: u32, sample_mid: bool) -> Result<(), String> {
		self.engine().run_script("::World.State.setAutoPause(false); ::World.State.setPause(false);", "unpause")?;
		self.engine()
			.run_script(&ENGAGE.replace("{n}", &n.to_string()), "engage")
			.map_err(|e| format!("engaging battle {n}: {e}"))?;
		let target = player::eval_string(self.engine(), "::__inventoryTarget")?.unwrap_or_default();
		self.until_page(SETTLE_MS, 3000, "the battle starting", IN_TACTICAL)?;
		if sample_mid {
			self.frames(BATTLE_MS, 300, DRAW_EVERY)?;
			self.frames(SETTLE_MS, 30, 1)?;
			self.sample(&format!("in battle {n}"))?;
		}
		self.fight_out(&format!("{n}. {target}"))
	}

	/// Fight the battle under way to its end (its enemies weakened, and the company healed,
	/// after a while and then every [`WEAKEN_EVERY`] frames).
	fn fight_out(&mut self, label: &str) -> Result<(), String> {
		self.frames(BATTLE_MS, 300, DRAW_EVERY)?;
		let mut ended = Err(String::new());
		for _ in 0..BATTLE_FRAMES / WEAKEN_EVERY {
			if self.holds(IN_TACTICAL)? {
				self.engine().run_script(WEAKEN, "weaken the enemies")?;
			}
			ended = self.until(BATTLE_MS, WEAKEN_EVERY, "the battle ending", BACK_IN_WORLD);
			if ended.is_ok() {
				break;
			}
		}
		if let Err(e) = ended {
			let stall = player::eval_string(self.engine(), STALL).ok().flatten().unwrap_or_default();
			let units = player::eval_string(self.engine(), UNITS).ok().flatten().unwrap_or_default();
			let stalled = format!("{e} (in all, {BATTLE_FRAMES} frames); {stall}; {units}");
			// The company flees, as a player would from a battle that won't end, and the
			// session goes on.
			eprintln!("{label}: stalled, the company flees: {stalled}");
			self.engine().run_script("::Tactical.State.flee();", "flee")?;
			self.until(BATTLE_MS, 6000, "the battle ending after fleeing", BACK_IN_WORLD)
				.map_err(|e| format!("{e}; before fleeing: {stalled}"))?;
			self.battles.push(format!("{label}: stalled, fled"));
			return self.frames(SETTLE_MS, 120, 1);
		}
		let result = player::eval_i64(self.engine(), "::__lastCombatResult")?.unwrap_or(-1);
		self.battles.push(format!("{label}: result {result}"));
		self.frames(SETTLE_MS, 120, 1)
	}
}

fn mb(bytes: u64) -> String {
	format!("{:.0}", bytes as f64 / 1_048_576.0)
}

/// The session's samples and battles, and why it stopped early if it did.
fn run(gpu: &Gpu, o: &Options) -> Result<(Vec<Sample>, Vec<String>, Option<String>), String> {
	let boot = bb_app::game::Boot {
		data_dir: &o.data,
		start: bb_app::game::Start::NewCampaign(o.seed.clone()),
		mods: o.mods,
		sound: false,
		world_size: None,
		mode: VideoMode::DEFAULT,
		settings: None,
		devtools: o.devtools,
		// The saves (and the mods' storages) stay in memory: the install's own are never touched.
		saves: None,
		cef_log: None,
		store: Default::default(),
		update_check: None,
		display: None,
	};
	let mut game = Game::start(gpu, &boot)?;
	game.track_texture_usage();
	let io = |e: std::io::Error| e.to_string();
	let files = GameFiles::open(harness::data_archives(&o.data).map_err(io)?).map_err(io)?;
	let sound = Rc::new(RefCell::new(KiraOut::silent(std::sync::Arc::new(files), 0.3)?));
	game.front().engine().audio().set_out(Box::new(SilentOut {
		out: sound.clone(),
		last_ms: None,
	}));
	let (width, height) = game.screen();
	let target = RenderTarget::new(gpu, width, height);
	let mut s = Session {
		game,
		gpu,
		target,
		sound,
		samples: Vec::new(),
		battles: Vec::new(),
		until: o.until.clone(),
	};
	let stopped = play(&mut s, o).err().filter(|e| e != UNTIL);
	if o.hold > 0 {
		// Frames go on at their real pace, so the page answers DevTools.
		eprintln!("holding for {} s", o.hold);
		let until = std::time::Instant::now() + std::time::Duration::from_secs(o.hold);
		while std::time::Instant::now() < until {
			let _ = s.frame(SETTLE_MS, 1, DRAW_EVERY);
			std::thread::sleep(std::time::Duration::from_millis(16));
		}
	}
	Ok((s.samples, s.battles, stopped))
}

/// A script error's call stack, to the log (the page's game has no print handler).
const ERROR_STACK: &str = r#"
seterrorhandler(function( _e ) {
	local s = "SCRIPT ERROR " + _e;
	for (local i = 2; i < 30; i++)
	{
		local si = getstackinfos(i);
		if (si == null) break;
		s += "\n    at " + si.func + " (" + si.src + ":" + si.line + ")";
	}
	::logError(s);
});
"#;

fn play(s: &mut Session, o: &Options) -> Result<(), String> {
	s.engine().run_script(ERROR_STACK, "error stacks")?;
	harness::install_autoplay(s.engine())?;
	s.until_world("the campaign's world")?;
	s.frames(SETTLE_MS, 600, 1)?;
	s.sample("world, fresh")?;
	let mid = |n: u32| n == 1 || n == o.battles;
	for n in 1..=o.battles {
		s.battle(n, mid(n))?;
		if o.every_battle || n == 1 || n == o.battles / 2 || n == o.battles {
			s.sample(&format!("after battle {n}"))?;
		}
	}
	// Days on the world map, the time running as fast as frames allow. The parties leave
	// the company be (battles are measured above), so the days pass.
	s.engine().run_script(
		"::World.State.getPlayer().setAttackableByAI(false); ::World.State.setAutoPause(false); ::World.State.setPause(false); ::World.setSpeedMult(3.0);",
		"run",
	)?;
	let day = |s: &Session| player::eval_i64(s.engine(), "::World.getTime().Days").map(|d| d.unwrap_or(0));
	let until_day = day(s)? + i64::from(o.days);
	let mut waited = 0;
	let mut met = 0;
	while day(s)? < until_day {
		if s.holds(IN_TACTICAL)? {
			// A hostile party caught the company (the scripted player engages).
			met += 1;
			s.fight_out(&format!("on the road {met}"))?;
			s.engine().run_script("::World.setSpeedMult(3.0);", "run")?;
		}
		s.frames(DAYS_MS, 200, DRAW_EVERY)?;
		s.engine().run_script("::World.State.setPause(false);", "unpause")?;
		waited += 200;
		if waited % 2000 == 0 {
			let state = player::eval_string(s.engine(), WORLD_STATE)?.unwrap_or_default();
			eprintln!("days: {waited} frames, {state}");
			if waited >= 40_000 {
				return Err(format!("the days not passing: {state}"));
			}
		}
	}
	s.frames(SETTLE_MS, 60, 1)?;
	let today = day(s)?;
	s.sample(&format!("world, day {today}"))?;
	s.engine().run_script("::World.State.exitGame();", "quit to the menu")?;
	s.until_page(SETTLE_MS, 6000, "the main menu", AT_MENU)?;
	s.frames(SETTLE_MS, 300, 1)?;
	s.sample("main menu, after quitting")?;
	#[cfg(feature = "ui")]
	{
		// Start once the last loading screen has faded out, and a moment after, in real
		// time (the page's animations run on its own clock; the frames here don't wait).
		// A show within ~400 ms of a hide is lost, in the original too.
		s.until_page(SETTLE_MS, 6000, "the menu's loading screen fading out", "!::LoadingScreen.isVisible() && !::LoadingScreen.isAnimating()")?;
		let settle = std::time::Instant::now();
		while settle.elapsed().as_secs_f64() < 0.3 {
			s.frames(SETTLE_MS, 1, 1)?;
			std::thread::sleep(std::time::Duration::from_millis(16));
		}
		s.game.front().session().start_campaign(&format!("{}-2", o.seed))?;
	}
	#[cfg(not(feature = "ui"))]
	{
		harness::prepare_new_campaign(s.engine(), &format!("{}-2", o.seed))?;
		harness::start_new_campaign(s.engine())?;
	}
	s.until_world("the second campaign's world")?;
	s.frames(SETTLE_MS, 600, 1)?;
	s.sample("second campaign, fresh")?;
	Ok(())
}

/// `--engine-only`: the engine headless, with the install's mods if `--mods`, at the main
/// menu and on a new campaign's world (MSU's new-campaign settings kept as they are).
fn engine_only(o: &Options) -> Result<String, String> {
	let io = |e: std::io::Error| e.to_string();
	let mut archives = harness::data_archives(&o.data).map_err(io)?;
	let mods = if o.mods { harness::mod_archives(&o.data).map_err(io)? } else { Vec::new() };
	archives.extend(mods.iter().cloned());
	let engine = harness::open_mounted(&o.data, archives, |_| {}).map_err(io)?;
	harness::boot_menu(&engine)?;
	let run = |frames: u32| (0..frames).try_for_each(|_| harness::run_frame(&engine, SETTLE_MS));
	let flag = |cond: &str| player::eval_i64(&engine, &format!("({cond}) ? 1 : 0")).map(|v| v == Some(1));
	let mut rows: Vec<(String, Vec<ProcessMemory>, u64, u64)> = Vec::new();
	let mut sample = |point: &str| {
		heap_profile(rows.len(), point);
		let m = bb_engine::heap::usage().unwrap_or_default();
		rows.push((point.to_string(), stats::tree_memory().unwrap_or_default(), m.in_use as u64, m.free as u64));
	};
	run(120)?;
	sample("main menu");
	const MENU: &str = "::RootState.get(\"MainMenuState\").m.MainMenuScreen";
	engine.run_script(&format!("{MENU}.getMainMenuModule().onNewCampaignButtonPressed();"), "new campaign")?;
	run(1)?;
	engine.run_script(
		&format!(
			"{MENU}.getNewCampaignMenuModule().onStartButtonPressed({{
				companyName = \"Measured\", banner = \"banner_04\",
				combatDifficulty = 1, economicDifficulty = 1, budgetDifficulty = 1,
				ironman = false, explorationDifficulty = false,
				lateGameCrisis = ::Const.World.GreaterEvilType.Random, permanentDestruction = false,
				seed = \"{}\", origin = \"scenario.early_access\"
			}});",
			o.seed
		),
		"start",
	)?;
	let msu_settings = "(\"MSU\" in ::getroottable()) && ::MSU.SettingsScreen.isVisible()";
	for _ in 0..6000 {
		if flag(IN_WORLD)? {
			break;
		}
		if flag(msu_settings)? {
			engine.run_script("::MSU.SettingsScreen.onSaveButtonPressed({});", "MSU settings")?;
		}
		run(1)?;
	}
	if !flag(IN_WORLD)? {
		return Err("the campaign didn't start".into());
	}
	engine.run_script("::World.State.setAutoPause(false); ::World.State.setPause(false);", "unpause")?;
	run(600)?;
	sample("world, fresh");
	let mut r = format!(
		"# Memory inventory, the engine alone\n\nHeadless (no renderer, no page), seed `{}`, {} mods. MB.\n\n| | {} |\n|---|{}\n",
		o.seed,
		mods.len(),
		rows.iter().map(|r| r.0.as_str()).collect::<Vec<_>>().join(" | "),
		"---:|".repeat(rows.len())
	);
	let line = |name: &str, f: &dyn Fn(&(String, Vec<ProcessMemory>, u64, u64)) -> String| {
		format!("| {name} | {} |\n", rows.iter().map(f).collect::<Vec<_>>().join(" | "))
	};
	r += &line("PSS", &|r| mb(r.1.iter().map(ProcessMemory::pss).sum()));
	for k in [MappingKind::Heap, MappingKind::Anonymous, MappingKind::Files] {
		r += &line(&format!("&nbsp;&nbsp;{}", k.name()), &|r| {
			mb(r.1.iter().flat_map(|p| p.kinds.iter()).filter(|(kk, _)| *kk == k).map(|(_, b)| *b).sum())
		});
	}
	r += &line("malloc: in use", &|r| mb(r.2));
	r += &line("malloc: held free", &|r| mb(r.3));
	Ok(r)
}

/// The CEF processes' role groups, in the report's order.
fn group(role: &str) -> &'static str {
	match role {
		"game" => "game",
		r if r.starts_with("renderer") => "CEF renderer",
		_ => "CEF other",
	}
}

fn report(o: &Options, samples: &[Sample], battles: &[String], stopped: Option<&str>) -> String {
	let mut r = String::new();
	let ui = cfg!(feature = "ui");
	r += "# Memory inventory\n\n";
	r += &format!(
		"`memory_inventory` ({} UI), seed `{}`, {} battles, {} days. MB throughout (2^20 bytes). PSS counts a page several processes share once, split between them.\n\n",
		if ui { "with the" } else { "without the" },
		o.seed,
		o.battles,
		o.days
	);
	if let Some(e) = stopped {
		r += &format!("**The session stopped early:** {e}\n\n");
	}
	let header: Vec<&str> = samples.iter().map(|s| s.point.as_str()).collect();
	r += &format!("| | {} |\n|---|{}\n", header.join(" | "), "---:|".repeat(header.len()));
	let mut row = |name: &str, f: &dyn Fn(&Sample) -> String| {
		r += &format!("| {name} | {} |\n", samples.iter().map(f).collect::<Vec<_>>().join(" | "));
	};
	let pss = |s: &Sample, g: &str| s.processes.iter().filter(|p| group(&p.role) == g).map(ProcessMemory::pss).sum::<u64>();
	let kind = |s: &Sample, g: &str, k: MappingKind| {
		s.processes
			.iter()
			.filter(|p| group(&p.role) == g)
			.flat_map(|p| p.kinds.iter())
			.filter(|(kk, _)| *kk == k)
			.map(|(_, b)| *b)
			.sum::<u64>()
	};
	row("**RAM, all processes (PSS)**", &|s| format!("**{}**", mb(s.processes.iter().map(ProcessMemory::pss).sum())));
	row("**GPU memory (VRAM + GTT)**", &|s| format!("**{}**", mb(s.processes.iter().map(|p| p.vram + p.gtt).sum())));
	row("processes", &|s| s.processes.len().to_string());
	row("game process", &|s| mb(pss(s, "game")));
	for k in [
		MappingKind::Heap,
		MappingKind::Anonymous,
		MappingKind::Libcef,
		MappingKind::GpuDriver,
		MappingKind::Shared,
		MappingKind::Files,
	] {
		row(&format!("&nbsp;&nbsp;{}", k.name()), &|s| mb(kind(s, "game", k)));
	}
	row("&nbsp;&nbsp;malloc: in use", &|s| mb(s.heap_used));
	row("&nbsp;&nbsp;malloc: held free (main arena)", &|s| mb(s.heap_free));
	if ui {
		row("CEF renderer", &|s| mb(pss(s, "CEF renderer")));
		for k in [MappingKind::V8, MappingKind::PartitionAlloc, MappingKind::Libcef, MappingKind::Shared, MappingKind::Anonymous] {
			row(&format!("&nbsp;&nbsp;{}", k.name()), &|s| mb(kind(s, "CEF renderer", k)));
		}
		row("CEF other processes", &|s| mb(pss(s, "CEF other")));
	}
	row("wgpu allocations / reserved blocks", &|s| format!("{} / {}", mb(s.gpu_allocated), mb(s.gpu_reserved)));
	// What the game process's GPU memory holds beyond wgpu's blocks: the driver's own,
	// and the page's (Chromium's GPU work runs in this process; on amdgpu its GL context
	// shares our device, so its memory counts in our DRM client).
	row("&nbsp;&nbsp;game process GPU beyond wgpu's blocks", &|s| {
		let game: u64 = s.processes.iter().filter(|p| p.role == "game").map(|p| p.vram + p.gtt).sum();
		mb(game.saturating_sub(s.gpu_reserved))
	});
	row("renderer: pages of frames (frames)", &|s| format!("{} ({})", s.renderer.pages, s.renderer.frames));
	row("&nbsp;&nbsp;their bytes (mips counted)", &|s| mb(s.renderer.page_bytes));
	row("renderer: textures (count)", &|s| s.renderer.textures.len().to_string());
	row("&nbsp;&nbsp;their bytes (mips counted)", &|s| mb(s.renderer.texture_bytes()));
	row("&nbsp;&nbsp;error.png stand-ins", &|s| s.renderer.textures.iter().filter(|t| t.stand_in).count().to_string());
	// What the frames drew since the last point, as RGBA8 without mips, and what's loaded
	// but wasn't drawn in that time.
	let drawn = |s: &Sample| s.drawn.iter().map(|a| a.drawn_texels * 4).sum::<u64>();
	let never = |s: &Sample| {
		s.renderer
			.textures
			.iter()
			.filter(|t| !s.drawn.iter().any(|a| a.path == t.path && a.drawn_texels > 0))
			.map(|t| t.bytes)
			.sum::<u64>()
	};
	row("&nbsp;&nbsp;drawn since the last point (RGBA8, no mips)", &|s| mb(drawn(s)));
	row("&nbsp;&nbsp;loaded, not drawn since the last point", &|s| mb(never(s)));
	row("&nbsp;&nbsp;fog mask, UI page, own", &|s| mb(s.renderer.fog_mask + s.renderer.ui_page + s.renderer.own));
	row("tactical unit records (dead)", &|s| format!("{} ({})", s.units, s.dead_units));
	row("world entity records", &|s| s.world_entities.to_string());
	row("saves kept in memory (MB)", &|s| format!("{} ({})", s.saves.0, mb(s.saves.1 as u64)));
	row("sound bank: sounds", &|s| s.sounds.0.to_string());
	row("&nbsp;&nbsp;decoded (trimmed at 32)", &|s| mb(s.sounds.1 as u64));
	row("&nbsp;&nbsp;long sounds playing (their files)", &|s| mb(s.sounds.2 as u64));
	row("&nbsp;&nbsp;read ahead, not yet played", &|s| mb(s.sounds.3 as u64));
	row("sound voices (never started, ended)", &|s| format!("{} ({}, {})", s.voices.0, s.voices.1, s.voices.2));
	if ui {
		row("UI log lines", &|s| s.ui_log.map_or("-".to_string(), |n| n.to_string()));
	}
	r += "\nBattles (opponent, troops, `getCombatResult`):\n\n";
	for b in battles {
		r += &format!("- {b}\n");
	}
	if let Some(last) = samples.iter().max_by_key(|s| s.renderer.texture_bytes()) {
		r += &format!(
			"\nTextures the renderer holds at \"{}\" (the most), and the share of each the frames drew since the point before:\n\n| texture | size | MB | drawn | mips sampled |\n|---|---|---:|---:|---|\n",
			last.point
		);
		for t in &last.renderer.textures {
			let stand_in = if t.stand_in { " (error.png stand-in)" } else { "" };
			let use_ = last.drawn.iter().find(|a| a.path == t.path);
			let share = use_.map_or(0.0, |a| a.drawn_texels as f64 / (f64::from(t.width) * f64::from(t.height)) * 100.0);
			let mips = use_.map_or("-", |a| if a.mips { "yes" } else { "no" });
			r += &format!(
				"| `{}`{stand_in} | {}×{} | {:.1} | {share:.0}% | {mips} |\n",
				t.path,
				t.width,
				t.height,
				t.bytes as f64 / 1_048_576.0
			);
		}
	}
	if let Some(last) = samples.last() {
		r += &format!("\nwgpu's allocations by label at \"{}\" (the largest):\n\n| label | MB |\n|---|---:|\n", last.point);
		for (name, bytes) in last.gpu_by_name.iter().take(40) {
			r += &format!("| `{}` | {:.1} |\n", if name.is_empty() { "(unnamed)" } else { name }, *bytes as f64 / 1_048_576.0);
		}
	}
	for s in samples.iter().filter(|s| s.point == "world, fresh" || s.point.starts_with("world, day")) {
		r += &format!("\nProcesses at \"{}\":\n\n| process | PSS | by kind | VRAM | GTT |\n|---|---:|---|---:|---:|\n", s.point);
		for p in &s.processes {
			let kinds: Vec<String> = p.kinds.iter().filter(|(_, b)| *b >= 1 << 20).map(|(k, b)| format!("{} {}", k.name(), mb(*b))).collect();
			r += &format!("| {} | {} | {} | {} | {} |\n", p.role, mb(p.pss()), kinds.join(", "), mb(p.vram), mb(p.gtt));
		}
	}
	r
}

fn main() -> ExitCode {
	#[cfg(feature = "ui")]
	if let Some(code) = bb_ui::host::run_subprocess() {
		return ExitCode::from(code.clamp(0, 255) as u8);
	}
	// Chromium paints the page through a display connection: a headless gamescope's.
	#[cfg(feature = "ui")]
	return bb_ui::host::with_display(run_main);
	#[cfg(not(feature = "ui"))]
	run_main()
}

fn run_main() -> ExitCode {
	// As bb sets up the C allocator, for itself and the browser's processes.
	bb_app::platform::tune_allocator();
	let result = (|| -> Result<(), String> {
		let o = options()?;
		if !Path::new(&o.data).exists() {
			return Err(format!("{} not present", o.data.display()));
		}
		if o.engine_only {
			let report = engine_only(&o)?;
			return match &o.out {
				Some(path) => std::fs::write(path, report).map_err(|e| format!("{}: {e}", path.display())),
				None => Ok(print!("{report}")),
			};
		}
		let gpu = Gpu::headless_with(bb_app::front::GPU_FEATURES)?;
		let (samples, battles, stopped) = run(&gpu, &o)?;
		if let Some(e) = &stopped {
			eprintln!("the session stopped early: {e}");
		}
		let report = report(&o, &samples, &battles, stopped.as_deref());
		match &o.out {
			Some(path) => std::fs::write(path, report).map_err(|e| format!("{}: {e}", path.display()))?,
			None => print!("{report}"),
		}
		Ok(())
	})();
	match result {
		Ok(()) => ExitCode::SUCCESS,
		Err(e) => {
			eprintln!("error: {e}");
			ExitCode::FAILURE
		}
	}
}
