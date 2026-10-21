//! `bb`'s window, run in a headless gamescope (muted, so nothing shows or sounds): it
//! starts at the main menu, exits cleanly, and the scripts' video mode is put on it as they set it. Skips
//! cleanly without gamescope or the Steam install, or when gamescope can't start (no
//! display, no runtime folder). Each run waits its turn behind the probes' games
//! ([`bb_engine::game_queue`]) and gives the display up when it ends (a runner that
//! queued once hands its turn down, `BB_GAME_QUEUE_TURN`). Every run logs into its own
//! folder (`--logs`).

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
fn run_bb(dir: &Path, size: (u32, u32), args: &[&str]) -> Option<Run> {
	run_bb_with(dir, size, args, &[])
}

/// [`run_bb`] with `env` set for `bb` too.
fn run_bb_with(dir: &Path, size: (u32, u32), args: &[&str], env: &[(&str, &str)]) -> Option<Run> {
	run_bb_at(dir, size, None, args, env)
}

/// [`run_bb_with`] on a display refreshing `refresh` times a second (gamescope's own
/// default otherwise).
fn run_bb_at(dir: &Path, (width, height): (u32, u32), refresh: Option<u32>, args: &[&str], env: &[(&str, &str)]) -> Option<Run> {
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
		.args(["-W", &width.to_string(), "-H", &height.to_string()])
		.args(refresh.map(|r| ["-r".to_string(), r.to_string()]).into_iter().flatten())
		.arg("--")
		.args([
			"sh",
			"-c",
			r#": > "$BB_TEST_STARTED"; "$0" "$@" 2>"$BB_TEST_STDERR"; echo $? > "$BB_TEST_STATUS""#,
		])
		.arg(bb_binary())
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
		// So a run that hangs can have its stacks read (`dump_stacks`).
		.env("BB_ALLOW_TRACE", "1")
		.envs(env.iter().copied())
		.env("BB_TEST_STARTED", &started)
		.env("BB_TEST_STATUS", &status)
		.env("BB_TEST_STDERR", &stderr)
		.stdout(log.try_clone().expect("gamescope's log"))
		.stderr(log)
		.spawn()
		.expect("run gamescope");
	// bb's end is its status, not gamescope's: under load gamescope's own teardown (its
	// Xwayland and the rest) has taken minutes after bb was gone. Once bb is, gamescope
	// gets a while to go too, and is then killed.
	let deadline = Instant::now() + Duration::from_secs(300);
	let mut ended: Option<Instant> = None;
	while child.try_wait().expect("gamescope's status").is_none() {
		let now = Instant::now();
		if ended.is_none() && std::fs::read_to_string(&status).is_ok_and(|s| s.ends_with('\n')) {
			ended = Some(now);
		}
		if ended.is_some_and(|at| now > at + Duration::from_secs(20)) {
			eprintln!("bb ended, but gamescope didn't within 20 s: killed");
			kill_tree(&mut child);
			break;
		}
		if ended.is_none() && now > deadline {
			let stacks = dir.join("stacks.txt");
			dump_stacks(child.id(), &stacks);
			kill_tree(&mut child);
			panic!("bb didn't exit within 5 minutes; its threads' stacks in {}", stacks.display());
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

/// The `bb` to run: the one `BB_TEST_BIN` names, else this build's. A runner that builds
/// the workspace with and without the `ui` feature names the matching one, since the
/// second build replaces the first's `target/…/bb`.
fn bb_binary() -> PathBuf {
	std::env::var_os("BB_TEST_BIN").map_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_bb")), PathBuf::from)
}

/// Kill gamescope and what's under it (bb, its browser processes, gamescope's Xwayland),
/// which killing gamescope alone leaves running.
fn kill_tree(gamescope: &mut std::process::Child) {
	let left = descendants(gamescope.id());
	let _ = gamescope.kill();
	let _ = gamescope.wait();
	for pid in left {
		let _ = Command::new("kill").args(["-9", &pid.to_string()]).status();
	}
}

/// The processes under `root` (its children, theirs, ...), from `/proc`.
fn descendants(root: u32) -> Vec<u32> {
	let mut parents = Vec::new();
	for entry in std::fs::read_dir("/proc").into_iter().flatten().flatten() {
		let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
		let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else { continue };
		// `pid (comm) state ppid ...`; comm may hold spaces and parentheses.
		let Some(ppid) = stat.rsplit_once(')').and_then(|(_, rest)| rest.split_whitespace().nth(1)?.parse::<u32>().ok()) else {
			continue;
		};
		parents.push((pid, ppid));
	}
	let mut found = vec![root];
	let mut at = 0;
	while at < found.len() {
		let parent = found[at];
		found.extend(parents.iter().filter(|(_, ppid)| *ppid == parent).map(|(pid, _)| *pid));
		at += 1;
	}
	found.remove(0);
	found
}

/// For a run that hangs: every thread of gamescope and of the processes under it (bb,
/// its browser processes), with its state and kernel wait channel, and the user-space
/// stacks `eu-stack` reads (bb allows it, `BB_ALLOW_TRACE`; gamescope doesn't), into
/// `out`.
fn dump_stacks(gamescope: u32, out: &Path) {
	use std::fmt::Write as _;
	let mut text = String::new();
	for pid in std::iter::once(gamescope).chain(descendants(gamescope)) {
		let task = |file: &str| std::fs::read_to_string(format!("/proc/{pid}/{file}")).unwrap_or_default();
		let cmdline = task("cmdline").replace('\0', " ");
		let _ = writeln!(text, "=== {pid}: {}", cmdline.chars().take(300).collect::<String>());
		for thread in std::fs::read_dir(format!("/proc/{pid}/task")).into_iter().flatten().flatten() {
			let read = |file: &str| std::fs::read_to_string(thread.path().join(file)).unwrap_or_default();
			let stat = read("stat");
			let state = stat.rsplit_once(')').and_then(|(_, r)| r.split_whitespace().next()).unwrap_or("?").to_string();
			let _ = writeln!(text, "  thread {} {:?} {state} wchan {}", thread.file_name().to_string_lossy(), read("comm").trim(), read("wchan"));
		}
		match Command::new("eu-stack").args(["-p", &pid.to_string()]).stderr(Stdio::piped()).output() {
			Ok(output) => {
				text.push_str(&String::from_utf8_lossy(&output.stdout));
				text.push_str(&String::from_utf8_lossy(&output.stderr));
			}
			Err(e) => {
				let _ = writeln!(text, "  (no eu-stack: {e})");
			}
		}
	}
	let _ = std::fs::write(out, text);
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

/// A frame whose present never reaches the compositor (one that came back out of date,
/// which wgpu drops silently) leaves winit waiting on Wayland's frame callback for good:
/// under load, `bb` stood still there at once in several of these tests ("didn't exit",
/// its main thread idle in winit's poll). It draws on without the callback and ends.
#[test]
fn bb_draws_on_after_a_frame_the_compositor_never_saw() {
	let dir = run_dir("unpresented");
	let (settings, shot) = (dir.join("settings.cfg"), dir.join("shot.png"));
	let args = ["--new-campaign", "--settings", settings.to_str().unwrap(), "--shot", "60", shot.to_str().unwrap()];
	let Some(run) = run_bb_with(&dir, (1280, 720), &args, &[("BB_TEST_UNPRESENTED", "20")]) else { return };
	assert_eq!(run.status, "0", "{}", run.stderr);
	assert!(run.stderr.contains("frame 20 not presented"), "{}", run.stderr);
	assert!(shot.exists(), "the shot 40 frames after it");
}

/// The scripts quit (back to the main menu, then its Quit, which finishes their last
/// state; Escape there does the same): `bb` ends, as the original does, rather than
/// going on drawing nothing.
#[test]
fn bb_ends_when_the_scripts_quit() {
	let dir = run_dir("quit");
	let settings = dir.join("settings.cfg");
	// Back to the main menu once the world is up and out of its loading screen
	// (worldgen takes a while), as the in-game menu's Quit does; then the main menu's
	// Quit once the world is gone. Both are tried every 30 frames, and do nothing until
	// their moment.
	let exit = r#"if (!("__exited" in getroottable()) && ("State" in ::World) && ::World.State != null && ::World.State.m.WorldScreen != null && !::World.State.isInLoadingScreen()) { ::__exited <- true; ::World.State.m.MenuStack.popAll(true); ::World.State.exitGame(); }"#;
	let quit = r#"local menu = ::RootState.get("MainMenuState"); if (("__exited" in getroottable()) && ::RootState.get("WorldState") == null && menu != null) menu.main_menu_module_onQuitPressed();"#;
	let exits: Vec<String> = (1..=80).map(|i| (i * 30).to_string()).collect();
	let quits: Vec<String> = (1..=100).map(|i| (i * 60).to_string()).collect();
	let mut args = vec!["--new-campaign", "--settings", settings.to_str().unwrap()];
	for frame in &exits {
		args.extend(["--at", frame.as_str(), exit]);
	}
	for frame in &quits {
		args.extend(["--at", frame.as_str(), quit]);
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
	// scheduled events end with it), where `quit` (tried every 30 frames) Quits.
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
	let frames: Vec<String> = (1..=400).map(|i| (i * 30).to_string()).collect();
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
	if bytes != first {
		let at = bytes.iter().zip(&first).position(|(a, b)| a != b).unwrap_or(bytes.len().min(first.len()));
		let around = |b: &[u8]| String::from_utf8_lossy(&b[at.saturating_sub(200)..(at + 200).min(b.len())]).into_owned();
		panic!("the first log, not intact: {} vs {} bytes, from {at}:\n{}\n---\n{}", bytes.len(), first.len(), around(&bytes), around(&first));
	}
	eprintln!("the first log: {} bytes, zipped {}", first.len(), std::fs::metadata(zip).unwrap().len());
	if cfg!(feature = "ui") {
		assert!(kept.iter().any(|p| p.file_name().unwrap().to_string_lossy().starts_with("cef-")), "{kept:?}");
	}
}

/// The game steps once per refresh of the display, whichever frames are presented: on a
/// 180 Hz and a 60 Hz display, at the main menu with the page (nothing there changes
/// while idle), with frames presented changed or not:
///
/// - two in a row of every seven, as a mouse moved now and then presents them with the
///   game's cursor. Paced from the shortest recent interval between presented frames,
///   the first frames after a pause (presented as fast as the swapchain had images free)
///   set the pace of the undrawn ones: here 330-350 undrawn frames a second at 180 Hz,
///   220-390 at 60 Hz.
/// - every other one, as the UI page's paints landed in a CSS fade over the whole page
///   at 2560x1358. With undrawn frames paced a refresh after their own start, the frame
///   after a presented one came at once (the display's queue had room): ~312 steps a
///   second at 180 Hz, ~114 at 60 Hz.
#[test]
fn the_game_steps_at_the_displays_rate_whichever_frames_are_presented() {
	if !cfg!(feature = "ui") {
		eprintln!("skipping: without the page, the game starts a campaign, whose world map draws every frame");
		return;
	}
	for (refresh, presented) in [(180u32, "2/7"), (60, "2/7"), (180, "1/2"), (60, "1/2")] {
		let dir = run_dir(&format!("pace_{refresh}_{}", presented.replace('/', "_of_")));
		let (settings, times) = (dir.join("settings.cfg"), dir.join("frame_times.txt"));
		let quit = r#"local menu = ::RootState.get("MainMenuState"); if (menu != null) menu.main_menu_module_onQuitPressed();"#;
		// About ten seconds at the display's rate.
		let end = (refresh * 10).to_string();
		let args = ["--settings", settings.to_str().unwrap(), "--at", &end, quit];
		let env = [("BB_FRAME_TIMES", times.to_str().unwrap()), ("BB_TEST_PRESENT_EVERY", presented)];
		let Some(run) = run_bb_at(&dir, (1280, 720), Some(refresh), &args, &env) else { return };
		assert_eq!(run.status, "0", "{}", run.stderr);
		// Each frame's interval since the last and whether it was presented; the first
		// few seconds (the page loading, the menu showing) left out.
		let frames: Vec<(f64, bool)> = std::fs::read_to_string(&times)
			.expect("the frame times")
			.lines()
			.filter_map(|line| {
				let field = |name: &str| line.split(' ').find_map(|f| f.strip_prefix(name)).and_then(|v| v.parse::<f64>().ok());
				Some((field("interval=")?, field("present=").is_some()))
			})
			.collect();
		let settled = &frames[frames.len() / 2..];
		let drawn = settled.iter().filter(|f| f.1).count();
		let rate = 1000.0 * settled.len() as f64 / settled.iter().map(|f| f.0).sum::<f64>();
		let undrawn: Vec<f64> = settled.windows(2).filter(|w| !w[0].1 && !w[1].1).map(|w| w[1].0).collect();
		let undrawn_rate = 1000.0 * undrawn.len() as f64 / undrawn.iter().sum::<f64>().max(f64::MIN_POSITIVE);
		eprintln!(
			"{refresh} Hz, {presented} presented: {rate:.1} steps a second ({} frames, {drawn} presented); \
			{} undrawn after undrawn, {undrawn_rate:.1} a second",
			settled.len(),
			undrawn.len()
		);
		let expected = f64::from(refresh);
		let (k, n) = presented.split_once('/').map(|(k, n)| (k.parse::<f64>().unwrap(), n.parse::<f64>().unwrap())).unwrap();
		let share = drawn as f64 / settled.len() as f64;
		assert!((share - k / n).abs() < 0.05, "{drawn} of {} frames presented", settled.len());
		assert!((rate - expected).abs() < expected * 0.03, "{rate:.1} steps a second on a {refresh} Hz display");
		if !undrawn.is_empty() {
			assert!((undrawn_rate - expected).abs() < expected * 0.08, "undrawn frames at {undrawn_rate:.1} a second on a {refresh} Hz display");
		}
	}
}
