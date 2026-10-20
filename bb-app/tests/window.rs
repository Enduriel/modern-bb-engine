//! `bb`'s window, run in a headless gamescope (muted, so nothing shows or sounds): it
//! starts at the main menu, exits cleanly, and the scripts' video mode is put on it as they set it. Skips
//! cleanly without gamescope or the Steam install, or when gamescope can't start (no
//! display, no runtime folder). Each run waits its turn behind the probes' games
//! ([`bb_engine::game_queue`]). Every run logs into its own folder (`--logs`).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use bb_engine::harness::STEAM_DATA_DIR;

/// One gamescope at a time.
static GAMESCOPE: Mutex<()> = Mutex::new(());

/// What a run left: its exit status and what it printed.
struct Run {
	status: String,
	stderr: String,
}

/// A folder of its own for the run called `name`, emptied.
fn run_dir(name: &str) -> PathBuf {
	let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("bb_window").join(name);
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).expect("a folder for the run");
	dir
}

/// Run `bb` with `args` (and `--mute`) in a headless gamescope of `width` × `height`,
/// or `None` to skip: no install, no gamescope, or a gamescope that couldn't start.
fn run_bb(dir: &Path, (width, height): (u32, u32), args: &[&str]) -> Option<Run> {
	if !Path::new(STEAM_DATA_DIR).exists() {
		eprintln!("skipping: {STEAM_DATA_DIR} not present");
		return None;
	}
	let gamescope = Command::new("gamescope").arg("--help").stdout(Stdio::null()).stderr(Stdio::null()).status();
	if gamescope.is_err() {
		eprintln!("skipping: no gamescope");
		return None;
	}
	let _one = GAMESCOPE.lock().unwrap_or_else(|e| e.into_inner());
	// Behind the probes' games, which run in a headless gamescope too: two at once can
	// hang one's teardown.
	let _turn = bb_engine::game_queue::Ticket::wait(Duration::from_secs(3600)).expect("a turn for gamescope");
	let (started, status, stderr) = (dir.join("started"), dir.join("status"), dir.join("stderr"));
	let log = std::fs::File::create(dir.join("gamescope.log")).expect("gamescope's log");
	// gamescope doesn't pass its child's status on: the shell writes it, and first that
	// it started at all. Its own Wayland display (`--expose-wayland`) stands in for the
	// desktop's, so the window opens there.
	let mut child = Command::new("gamescope")
		.args(["--backend", "headless", "--expose-wayland"])
		.args(["-W", &width.to_string(), "-H", &height.to_string(), "--"])
		.args([
			"sh",
			"-c",
			r#": > "$BB_TEST_STARTED"; "$0" "$@" 2>"$BB_TEST_STDERR"; echo $? > "$BB_TEST_STATUS""#,
		])
		.arg(env!("CARGO_BIN_EXE_bb"))
		.arg("--mute")
		// Tests never start Steam, even with its library beside the test's bb.
		.arg("--no-steam")
		// Its log goes to the run's folder, never the player's.
		.arg("--logs")
		.arg(dir.join("logs"))
		.args(args)
		// Saves (and mods' storages) stay in the run's folder unless it names another:
		// never the user's.
		.env("BB_SAVES_DIR", dir.join("saves"))
		.env("BB_TEST_STARTED", &started)
		.env("BB_TEST_STATUS", &status)
		.env("BB_TEST_STDERR", &stderr)
		.stdout(log.try_clone().expect("gamescope's log"))
		.stderr(log)
		.spawn()
		.expect("run gamescope");
	let deadline = Instant::now() + Duration::from_secs(300);
	while child.try_wait().expect("gamescope's status").is_none() {
		if Instant::now() > deadline {
			let _ = child.kill();
			let _ = child.wait();
			panic!("bb didn't exit within 5 minutes");
		}
		std::thread::sleep(Duration::from_millis(200));
	}
	if !started.exists() {
		let log = std::fs::read_to_string(dir.join("gamescope.log")).unwrap_or_default();
		let last = log.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("nothing");
		eprintln!("skipping: gamescope couldn't start bb (no display or runtime folder?); its last line: {last}");
		return None;
	}
	Some(Run {
		status: std::fs::read_to_string(&status).expect("bb's exit status").trim().to_string(),
		stderr: std::fs::read_to_string(&stderr).unwrap_or_default(),
	})
}

/// A PNG's width and height.
fn png_size(path: &Path) -> (u32, u32) {
	let bytes = std::fs::read(path).expect("the shot");
	let word = |at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap());
	(word(16), word(20))
}

/// For a player, `bb` with the page stops at the main menu, where they start or load a
/// campaign; without the page there's nothing to click there, so it starts one.
#[test]
fn bb_starts_at_the_main_menu_for_a_player() {
	let dir = run_dir("menu");
	let (settings, shot) = (dir.join("settings.cfg"), dir.join("shot.png"));
	let report = r#"::logWarning("started at the " + (::RootState.get("WorldState") == null && ::RootState.get("MainMenuState") != null ? "menu" : "world"));"#;
	let args = ["--settings", settings.to_str().unwrap(), "--at", "30", report, "--shot", "40", shot.to_str().unwrap()];
	let Some(run) = run_bb(&dir, (1280, 720), &args) else { return };
	assert_eq!(run.status, "0", "{}", run.stderr);
	let expected = if cfg!(feature = "ui") { "started at the menu" } else { "started at the world" };
	assert!(run.stderr.contains(expected), "{expected}:\n{}", run.stderr);
}

/// The window's teardown once crashed in libwayland under gamescope's Vulkan layer,
/// after the surface drawn to it was destroyed (see `Running::close`).
#[test]
fn bb_exits_cleanly() {
	let dir = run_dir("exit");
	let (settings, shot) = (dir.join("settings.cfg"), dir.join("shot.png"));
	let args = ["--new-campaign", "--settings", settings.to_str().unwrap(), "--shot", "3", shot.to_str().unwrap()];
	let Some(run) = run_bb(&dir, (1280, 720), &args) else { return };
	assert_eq!(run.status, "0", "bb's exit status (139: the crash at exit)\n{}", run.stderr);
	assert!(shot.exists(), "the shot taken before the exit");
}

/// The scripts quit (back to the main menu, then its Quit, which finishes their last
/// state; Escape there does the same): `bb` ends, as the original does, rather than
/// going on drawing nothing.
#[test]
fn bb_ends_when_the_scripts_quit() {
	let dir = run_dir("quit");
	let settings = dir.join("settings.cfg");
	// Back to the main menu once the world is up (worldgen takes a while), as the
	// in-game menu's Quit does; then the main menu's Quit once the world is gone.
	let exit = r#"if (!("__exited" in getroottable()) && ("State" in ::World) && ::World.State != null && ::World.State.m.WorldScreen != null) { ::__exited <- true; ::World.State.m.MenuStack.popAll(true); ::World.State.exitGame(); }"#;
	let quit = r#"local menu = ::RootState.get("MainMenuState"); if (("__exited" in getroottable()) && ::RootState.get("WorldState") == null && menu != null) menu.main_menu_module_onQuitPressed();"#;
	let mut args = vec!["--new-campaign", "--settings", settings.to_str().unwrap()];
	for frame in ["600", "1200", "1800", "2400"] {
		args.extend(["--at", frame, exit]);
	}
	for frame in ["3000", "3600", "4200", "4800"] {
		args.extend(["--at", frame, quit]);
	}
	let Some(run) = run_bb(&dir, (1280, 720), &args) else { return };
	assert_eq!(run.status, "0", "bb's exit status\n{}", run.stderr);
}

/// The scripts' video mode, set as the Options screen sets it, goes on the window at
/// once: a window resized (no smaller than the Options screen's smallest resolution,
/// and the scripts' mode follows the window), then borderless over the monitor; and the
/// mode they keep is kept in the settings file.
#[test]
fn the_video_mode_applies_live() {
	let dir = run_dir("live");
	let (settings, shot) = (dir.join("settings.cfg"), dir.join("shot.png"));
	let start = "video.width = 1280\nvideo.height = 800\nvideo.fullscreen = false\nvideo.borderless = false\n";
	std::fs::write(&settings, start).expect("the settings");
	let resize = "local m = ::Settings.getVideoMode(); m.Width = 800; m.Height = 600; \
		::Settings.setVideoMode(m); ::Settings.saveWithCustomVideoMode(m);";
	let report = "local m = ::Settings.getVideoMode(); ::logWarning(\"mode \" + m.Width + \"x\" + m.Height);";
	let borderless = "local m = ::Settings.getVideoMode(); m.Borderless = true; m.Width = 1600; m.Height = 900; \
		::Settings.setVideoMode(m);";
	let args = [
		"--new-campaign",
		"--settings",
		settings.to_str().unwrap(),
		"--at",
		"10",
		resize,
		"--at",
		"15",
		report,
		"--at",
		"20",
		borderless,
		"--shot",
		"30",
		shot.to_str().unwrap(),
	];
	let Some(run) = run_bb(&dir, (1600, 900), &args) else { return };
	assert_eq!(run.status, "0", "{}", run.stderr);
	let placements: Vec<&str> = run.stderr.lines().filter(|l| l.starts_with("video: ")).collect();
	assert!(placements[0].starts_with("video: 1280x800 Window { decorated: true, width: 1280, height: 800 }"), "{placements:?}");
	assert_eq!(placements[1..], ["video: 800x600 Window { decorated: true, width: 1024, height: 768 }", "video: 1600x900 Borderless"]);
	assert!(run.stderr.contains("mode 1024x768"), "the window kept at the smallest, the mode following it:\n{}", run.stderr);
	assert_eq!(png_size(&shot), (1600, 900), "borderless over the monitor");
	let kept = std::fs::read_to_string(&settings).expect("the settings");
	assert!(kept.contains("video.width = 800\n") && kept.contains("video.height = 600\n"), "the mode kept:\n{kept}");
	assert!(kept.contains("video.borderless = false\n"), "the borderless mode wasn't kept:\n{kept}");
}

/// A game that starts borderless over the monitor, with VSync off (a settings file
/// asking for both; the first run after a restart once hung there, after the swapchain
/// was made again for the borderless window), draws its frames and ends.
#[test]
fn bb_starts_borderless_and_draws() {
	let dir = run_dir("borderless");
	let (settings, shot) = (dir.join("settings.cfg"), dir.join("shot.png"));
	let start = "video.width = 1920\nvideo.height = 1080\nvideo.fullscreen = false\nvideo.borderless = true\nvideo.vsync = false\n";
	std::fs::write(&settings, start).expect("the settings");
	let args = ["--new-campaign", "--settings", settings.to_str().unwrap(), "--shot", "300", shot.to_str().unwrap()];
	let Some(run) = run_bb(&dir, (1920, 1080), &args) else { return };
	assert_eq!(run.status, "0", "{}", run.stderr);
	assert!(run.stderr.contains("video: 1920x1080 Borderless"), "{}", run.stderr);
	assert_eq!(png_size(&shot), (1920, 1080), "borderless over the monitor");
}

/// `bb --mods --new-campaign` starts the campaign once the mods are ready for it, and
/// reaches the world: with the page, as late as a player's click on Start could come,
/// after the mods' late JS has connected their screens (MSU's settings page, which a
/// new campaign shows first, used to throw "The screen 'null' does not have a
/// JSHandle" at boot). Its Save, pressed as a player would, goes on with the start.
/// Then back to the main menu and Quit. The saves (and the mods' storages) and the
/// settings go to the run's own folder. Skips without the mods it needs.
#[test]
fn bb_starts_a_modded_campaign_at_once() {
	let data = Path::new(STEAM_DATA_DIR);
	let mods = bb_engine::harness::mod_archives(data).unwrap_or_default();
	let has = |prefix: &str| mods.iter().any(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(prefix)));
	if !has("mod_modern_hooks") || !has("mod_msu") {
		eprintln!("skipping: Modern Hooks or MSU not installed");
		return;
	}
	let dir = run_dir("modded_campaign");
	let (settings, saves) = (dir.join("settings.cfg"), dir.join("saves"));
	std::fs::create_dir_all(&saves).expect("the saves folder");
	// Every 250 ms of real time: press MSU's Save on its new-campaign settings page once
	// it shows, note the world once it's up and go back to the main menu (the world's
	// scheduled events end with it), where `quit` (tried every 300 frames) Quits.
	let player = r#"
::__player <- { saved = false, world = false, exited = false };
::__playerTick <- function( _d ) {
	local p = ::__player;
	local up = ("State" in ::World) && ::World.State != null && ::World.State.getPlayer() != null && !::World.State.isInLoadingScreen();
	if (!p.saved && ("MSU" in ::getroottable()) && ::MSU.SettingsScreen.isVisible())
	{
		p.saved = true;
		::logWarning("player: MSU's new-campaign settings showed; Save");
		::MSU.SettingsScreen.onSaveButtonPressed({});
	}
	else if (!p.world && up)
	{
		p.world = true;
		::logWarning("player: reached the world");
		::World.State.m.MenuStack.popAll(true);
		::World.State.exitGame();
		p.exited = true;
	}
	if (!p.exited)
		::Time.scheduleEvent(::TimeUnit.Real, 250, ::__playerTick, null);
};
::Time.scheduleEvent(::TimeUnit.Real, 250, ::__playerTick, null);
"#;
	let quit = r#"local menu = ::RootState.get("MainMenuState"); if (("__player" in getroottable()) && ::__player.exited && ::RootState.get("WorldState") == null && menu != null) menu.main_menu_module_onQuitPressed();"#;
	let frames: Vec<String> = (1..=40).map(|i| (i * 300).to_string()).collect();
	let mut args = vec![
		"--mods",
		"--new-campaign",
		"--settings",
		settings.to_str().unwrap(),
		"--saves",
		saves.to_str().unwrap(),
		"--at",
		"1",
		player,
	];
	for frame in &frames {
		args.extend(["--at", frame.as_str(), quit]);
	}
	let Some(run) = run_bb(&dir, (1280, 720), &args) else { return };
	assert_eq!(run.status, "0", "bb's exit status\n{}", run.stderr);
	assert!(!run.stderr.contains("does not have a JSHandle"), "{}", run.stderr);
	assert!(run.stderr.contains("player: reached the world"), "{}", run.stderr);
	// With the page, the start shows MSU's page as the menu's Start does.
	if cfg!(feature = "ui") {
		assert!(run.stderr.contains("player: MSU's new-campaign settings showed"), "{}", run.stderr);
	}
}

/// `bb` logs to log.html in the original's format in the folder `--logs` names (with the
/// page, CEF to cef.log beside it); the next run keeps that log there, zipped, in `logs/`.
#[test]
fn bb_writes_its_log_and_keeps_the_last() {
	let dir = run_dir("logs");
	let (settings, shot, logs) = (dir.join("settings.cfg"), dir.join("shot.png"), dir.join("logs"));
	let lines = r#"::logWarning("marked <b>up</b>"); ::Time.scheduleEvent(::TimeUnit.Real, 1, function( _d ) { local probe = null; probe.foo(); }, null);"#;
	let args = ["--new-campaign", "--settings", settings.to_str().unwrap(), "--at", "3", lines, "--shot", "20", shot.to_str().unwrap()];
	let Some(run) = run_bb(&dir, (1280, 720), &args) else { return };
	assert_eq!(run.status, "0", "{}", run.stderr);
	let first = std::fs::read(logs.join("log.html")).expect("log.html");
	let html = String::from_utf8_lossy(&first);
	assert!(html.starts_with("<html><head><title>Battle Brothers 1.5.2.3</title><style type=\"text/css\">body{"), "{html}");
	assert!(html.contains("<div class=\"tag\">Core</div><div class=\"text\">Using write path: "), "{html}");
	assert!(html.contains("<div class=\"tag\">SQ</div><div class=\"text\">marked <b>up</b></div></div></div>"), "{html}");
	assert!(
		html.contains("<div class=\"tag\">Script Error</div><div class=\"text\">the index 'foo' does not exist</div></div><div class=\"stacktrace-container\">"),
		"{html}"
	);
	assert!(html.contains("<div class=\"valueVar\">probe = Null, _d = Null, this = Table</div>"), "{html}");
	assert!(html.ends_with("</div></body></html>"), "a clean exit ends the page: {html}");
	if cfg!(feature = "ui") {
		assert!(logs.join("cef.log").exists(), "cef.log beside log.html");
	}
	// The next run keeps the first's log, zipped alone.
	let Some(run) = run_bb(&dir, (1280, 720), &args) else { return };
	assert_eq!(run.status, "0", "{}", run.stderr);
	let kept: Vec<_> = std::fs::read_dir(logs.join("logs")).expect("logs/").flatten().map(|e| e.path()).collect();
	let zip = kept
		.iter()
		.find(|p| p.file_name().unwrap().to_string_lossy().starts_with("log-"))
		.unwrap_or_else(|| panic!("the first log's zip: {kept:?}"));
	let mut archive = zip::ZipArchive::new(std::fs::File::open(zip).unwrap()).unwrap();
	assert_eq!(archive.len(), 1);
	let mut entry = archive.by_index(0).unwrap();
	assert_eq!(entry.name(), "log.html");
	let mut bytes = Vec::new();
	std::io::Read::read_to_end(&mut entry, &mut bytes).unwrap();
	assert_eq!(bytes, first, "the first log, intact");
	eprintln!("the first log: {} bytes, zipped {}", first.len(), std::fs::metadata(zip).unwrap().len());
	if cfg!(feature = "ui") {
		assert!(kept.iter().any(|p| p.file_name().unwrap().to_string_lossy().starts_with("cef-")), "{kept:?}");
	}
}
