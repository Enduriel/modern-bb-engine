//! `bb`'s window, run in a headless gamescope (muted, so nothing shows or sounds): it
//! starts at the main menu, exits cleanly, and the scripts' video mode is put on it as they set it. Without
//! gamescope or the Steam install the tests fail, unless `BB_ALLOW_SKIPS=1` lets them skip
//! (machines without them); a gamescope that can't start fails them, with its log. Each run waits its turn behind the probes' games
//! ([`bb_engine::game_queue`]) and gives the display up when it ends (a runner that
//! queued once hands its turn down, `BB_GAME_QUEUE_TURN`). Every run logs into its own
//! folder (`--logs`).

use std::ffi::OsString;
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
/// or `None` to skip (only with skips allowed, `BB_ALLOW_SKIPS`): no install or no
/// gamescope.
fn run_bb(dir: &Path, size: (u32, u32), args: &[&str]) -> Option<Run> {
	run_bb_with(dir, size, args, &[])
}

/// [`run_bb`] with `env` set for `bb` (only: gamescope keeps the test's environment, so
/// a variable that picks a GPU or driver for `bb` can't leave gamescope without one).
fn run_bb_with(dir: &Path, size: (u32, u32), args: &[&str], env: &[(&str, &str)]) -> Option<Run> {
	run_bb_at(dir, size, None, args, env)
}

/// [`run_bb_with`] on a display refreshing `refresh` times a second (gamescope's own
/// default otherwise).
fn run_bb_at(dir: &Path, size: (u32, u32), refresh: Option<u32>, args: &[&str], env: &[(&str, &str)]) -> Option<Run> {
	if !Path::new(STEAM_DATA_DIR).exists() {
		bb_engine::harness::skip(format!("{STEAM_DATA_DIR} not present"));
		return None;
	}
	let mut command: Vec<OsString> = vec![bb_binary().into()];
	// Tests never start Steam, even with its library beside the test's bb. Its log goes
	// to the run's folder, never the player's. Vanilla unless the run asks for the mods
	// (`--mods`, after this: the last wins).
	command.extend(["--mute", "--no-steam", "--logs"].map(OsString::from));
	command.push(dir.join("logs").into());
	command.push("--no-mods".into());
	command.extend(args.iter().map(OsString::from));
	let run = run_in_gamescope(dir, size, refresh, &command, env)?;
	assert_saves_in(dir, &run);
	Some(run)
}

/// Run `command` (a program and its arguments, with `env` set for it) in a headless
/// gamescope of `width` × `height` refreshing `refresh` times a second, or `None` to
/// skip (only with skips allowed): no gamescope.
fn run_in_gamescope(dir: &Path, size: (u32, u32), refresh: Option<u32>, command: &[OsString], env: &[(&str, &str)]) -> Option<Run> {
	let gamescope = Command::new("gamescope").arg("--help").stdout(Stdio::null()).stderr(Stdio::null()).status();
	if let Err(e) = gamescope {
		bb_engine::harness::skip(format!("no gamescope ({e})"));
		return None;
	}
	let _one = GAMESCOPE.lock().unwrap_or_else(|e| e.into_inner());
	// Behind the probes' games, which run in a headless gamescope too: two at once can
	// hang one's teardown.
	let _turn = bb_engine::game_queue::Ticket::wait(Duration::from_secs(3600)).expect("a turn for gamescope");
	let run = match start_in_gamescope(dir, size, refresh, command, env) {
		Started::Ran(run) => run,
		// Seen when a run's environment reached gamescope too: a software Vulkan driver
		// chosen for `bb` (lavapipe) left gamescope no GPU it can drive. It no longer
		// does, so this is unexplained: one more try, saying so, then a failure.
		Started::NoBackend(log) => {
			eprintln!("gamescope couldn't create its backend; trying once more. Its log:\n{log}");
			match start_in_gamescope(dir, size, refresh, command, env) {
				Started::Ran(run) => run,
				Started::NoBackend(log) | Started::Failed(log) => panic!("gamescope couldn't start bb, twice; its log:\n{log}"),
			}
		}
		Started::Failed(log) => panic!("gamescope couldn't start bb; its log:\n{log}"),
	};
	Some(run)
}

/// Every run keeps its saves in its own folder `dir`, never the player's (`bb`'s default
/// is the original game's savegames folder, the player's real saves): each `saves:` line
/// `bb` printed names a folder in `dir`, or memory.
fn assert_saves_in(dir: &Path, run: &Run) {
	for line in run.stderr.lines().filter_map(|l| l.strip_prefix("saves: ")) {
		let ours = line.starts_with(&*dir.to_string_lossy()) || line.starts_with("in memory only");
		assert!(ours, "a run's saves outside its folder {}: {line}\n{}", dir.display(), run.stderr);
	}
}

/// How a gamescope run went: `bb` ran, or gamescope didn't get as far as starting it
/// (its log): it couldn't create its backend (no GPU it can drive), or anything else.
enum Started {
	Ran(Run),
	NoBackend(String),
	Failed(String),
}

/// One gamescope run of `command` for [`run_in_gamescope`], which holds the turn.
fn start_in_gamescope(dir: &Path, (width, height): (u32, u32), refresh: Option<u32>, command: &[OsString], env: &[(&str, &str)]) -> Started {
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
		// The program's own environment, through `env`: gamescope's stays the test's.
		.arg("env")
		.args(env.iter().map(|(k, v)| format!("{k}={v}")))
		.args(command)
		// Saves (and mods' storages) stay in the run's folder unless it names another
		// there: never the player's (checked, `assert_saves_in`).
		.env("BB_SAVES_DIR", dir.join("saves"))
		// So a run that hangs can have its stacks read (`dump_stacks`).
		.env("BB_ALLOW_TRACE", "1")
		// No message box that nobody is there to close, unless the run asks for one (its
		// `env`, set for bb over this).
		.env("BB_ERROR_DIALOG", "0")
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
		// Its own lines, without the dozens about the scripts it loads.
		let log: String = log.lines().filter(|l| !l.contains("scriptmgr:")).map(|l| format!("{l}\n")).collect();
		return if log.contains("Failed to create backend") { Started::NoBackend(log) } else { Started::Failed(log) };
	}
	Started::Ran(Run {
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

/// A script error once the game runs is reported and the game goes on, as the
/// original's: here vanilla's own, in the world state's `onFinish` when an event opened
/// as the world quit (it once ended `bb`, as "couldn't start", now and then under load:
/// see bb-engine's `quit_with_event_open.rs`). The event fires right after `exitGame()`,
/// as the world's update may fire one in the frames before the loading screen animates.
/// `bb` comes back to the main menu, quits from it and ends cleanly.
#[test]
fn bb_goes_on_after_a_script_error_as_the_world_quits() {
	let dir = run_dir("quit_event");
	let settings = dir.join("settings.cfg");
	// The menu stack emptied first, and the event screen's hiding over (with the page it
	// animates), then the quit: an event screen that is still animating opens no event.
	// (The world may have opened one of its own meanwhile: that one does as well.)
	let exit = r#"if (!("__exited" in getroottable()) && ("State" in ::World) && ::World.State != null && ::World.State.m.WorldScreen != null && !::World.State.isInLoadingScreen()) {
	if (!("__emptied" in getroottable())) { ::__emptied <- true; ::World.State.m.MenuStack.popAll(true); }
	else if (!::World.State.m.EventScreen.isAnimating()) { ::__exited <- true; ::World.State.exitGame(); local fired = ::World.Events.fire("event.early_access_scenario_intro"); ::logWarning("an event is open: " + (fired || ::World.State.m.MenuStack.hasBacksteps())); } }"#;
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
	assert!(run.stderr.contains("an event is open: true"), "{}", run.stderr);
	let reported = "Script Error: the index 'updateAssetsInformation' does not exist";
	assert_eq!(run.stderr.matches(reported).count(), 1, "the error reported once\n{}", run.stderr);
	let log = std::fs::read_to_string(dir.join("logs").join("log.html")).expect("log.html");
	assert!(log.contains("the index 'updateAssetsInformation' does not exist"), "in log.html too");
	assert!(!run.stderr.contains("couldn't start"), "{}", run.stderr);
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

/// The scripts' video mode switched between a window, fullscreen and back, with VSync
/// off and on again: each change configures the surface anew after frames were
/// presented (where Direct3D 12 has failed a game), and the game draws
/// on without the surface failing.
#[test]
fn the_video_mode_switches_fullscreen_and_vsync_live() {
	let dir = run_dir("switches");
	let (settings, shot) = (dir.join("settings.cfg"), dir.join("shot.png"));
	let start = "video.width = 1280\nvideo.height = 720\nvideo.fullscreen = false\nvideo.borderless = false\nvideo.vsync = true\n";
	std::fs::write(&settings, start).expect("the settings");
	let set = |change: &str| format!("local m = ::Settings.getVideoMode(); {change} ::Settings.setVideoMode(m);");
	let steps = [
		(10, set("m.Fullscreen = true; m.Width = 1600; m.Height = 900;")),
		(20, set("m.VSync = false;")),
		(30, set("m.Fullscreen = false; m.Width = 1280; m.Height = 720;")),
		(40, set("m.VSync = true;")),
	];
	let frames: Vec<String> = steps.iter().map(|(at, _)| at.to_string()).collect();
	let mut args = vec!["--new-campaign", "--settings", settings.to_str().unwrap()];
	for ((_, code), frame) in steps.iter().zip(&frames) {
		args.extend(["--at", frame.as_str(), code.as_str()]);
	}
	args.extend(["--shot", "60", shot.to_str().unwrap()]);
	let Some(run) = run_bb(&dir, (1600, 900), &args) else { return };
	assert_eq!(run.status, "0", "{}", run.stderr);
	let placements: Vec<&str> = run.stderr.lines().filter(|l| l.starts_with("video: ")).collect();
	assert_eq!(placements[1..], ["video: 1600x900 Borderless", "video: 1280x720 Window { decorated: true, width: 1280, height: 768 }"], "{placements:?}");
	assert!(!run.stderr.contains("the window's surface failed"), "{}", run.stderr);
	// The window no smaller than the Options screen's smallest resolution (1024x768).
	assert_eq!(png_size(&shot), (1280, 768), "back in a window");
}

/// A surface the system won't configure (as Direct3D 12 has refused one:
/// "Invalid surface", which wgpu's default handler made a panic) doesn't end the game:
/// it's logged, configured again each frame until it takes, and the game draws on. The
/// refusals here are wgpu's own, for an empty surface (`BB_TEST_SURFACE_FAILS`): the
/// window's first configures, then a frame's after the window is up.
#[test]
fn bb_rides_out_a_surface_the_system_refuses() {
	let dir = run_dir("surface_refused");
	let (settings, shot) = (dir.join("settings.cfg"), dir.join("shot.png"));
	let start = "video.width = 1280\nvideo.height = 720\nvideo.fullscreen = false\nvideo.borderless = false\nvideo.vsync = true\n";
	std::fs::write(&settings, start).expect("the settings");
	let args = ["--new-campaign", "--settings", settings.to_str().unwrap(), "--shot", "30", shot.to_str().unwrap()];
	let Some(run) = run_bb_with(&dir, (1280, 720), &args, &[("BB_TEST_SURFACE_FAILS", "12")]) else { return };
	assert_eq!(run.status, "0", "{}", run.stderr);
	let failed = run.stderr.matches("the window's surface failed (configure)").count();
	assert_eq!(failed, 1, "the first failure logged, the retries not:\n{}", run.stderr);
	// Both in log.html (the recovery, information, goes to stdout rather than stderr).
	let log = std::fs::read_to_string(dir.join("logs").join("log.html")).expect("log.html");
	assert!(log.contains("the window's surface failed (configure)"), "{log}");
	assert!(log.contains("the window's surface is configured again, after 12 failed attempts"), "{log}");
	// The window no smaller than 1024x768 (see `the_video_mode_applies_live`).
	assert_eq!(png_size(&shot), (1280, 768), "drawn on after it");
}

/// The window's surface configures while another thread submits (the UI page's copies of
/// the browser's paints, on CEF's thread): wgpu waits for the GPU to go idle before it
/// configures a surface, and fails the configure ("Failed to wait for GPU to come idle")
/// if a submission came in meanwhile. Under load that failed `bb`'s window for real, on
/// top of `bb_rides_out_a_surface_the_system_refuses`'s refusals. Configured
/// as `bb` does it (`Gpu::configure_surface`), it never fails; configured plainly, beside
/// it, it does, which shows the test reaches the race. The test runs itself again in a
/// headless gamescope, for a window.
#[cfg(target_os = "linux")]
#[test]
fn the_surface_configures_while_another_thread_submits() {
	const INNER: &str = "BB_TEST_SURFACE_RACE";
	if std::env::var_os(INNER).is_some() {
		return surface_race::run();
	}
	let dir = run_dir("surface_race");
	let name = "the_surface_configures_while_another_thread_submits";
	let exe = std::env::current_exe().expect("the test binary");
	let command = [exe.into(), name.into(), "--exact".into(), "--nocapture".into(), "--test-threads=1".into()];
	let Some(run) = run_in_gamescope(&dir, (640, 480), None, &command, &[(INNER, "1")]) else { return };
	let summary: Vec<&str> = run.stderr.lines().filter(|l| l.starts_with("surface race:")).collect();
	eprintln!("{}", summary.join("\n"));
	assert_eq!(run.status, "0", "{}", run.stderr);
	assert!(!summary.is_empty(), "the test didn't run in gamescope:\n{}", run.stderr);
}

/// [`the_surface_configures_while_another_thread_submits`] in its gamescope.
#[cfg(target_os = "linux")]
mod surface_race {
	use std::sync::Arc;
	use std::sync::atomic::{AtomicBool, Ordering};
	use std::time::{Duration, Instant};

	use bb_render::{Gpu, UiPage, UiRegion};
	use winit::platform::wayland::EventLoopBuilderExtWayland;

	/// Configures with the page submitting all along on another thread.
	const CONFIGURES: u32 = 500;
	/// The plain configures' limits, while none has failed yet.
	const PLAIN_TRIES: u32 = 5000;
	const PLAIN_TIME: Duration = Duration::from_secs(30);

	pub fn run() {
		let event_loop = winit::event_loop::EventLoop::builder()
			.with_wayland()
			.with_any_thread(true)
			.build()
			.expect("gamescope's Wayland display");
		#[allow(deprecated)]
		let window = Arc::new(
			event_loop
				.create_window(winit::window::Window::default_attributes().with_title("bb surface race"))
				.expect("a window"),
		);
		let display = Box::new(event_loop.owned_display_handle());
		let instance =
			wgpu::Instance::new(Gpu::platform_backends(wgpu::InstanceDescriptor::new_with_display_handle_from_env(display)));
		let surface = instance.create_surface(window.clone()).expect("a surface");
		let gpu = Gpu::for_surface_with(&instance, &surface, UiPage::FEATURES).expect("a GPU for the page");
		let page = UiPage::new(&gpu).expect("the page");
		let capabilities = gpu.surface_capabilities(&surface).expect("the surface's capabilities");
		let size = window.inner_size();
		let mut config = wgpu::SurfaceConfiguration {
			usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
			format: capabilities.formats[0],
			width: size.width.max(1),
			height: size.height.max(1),
			present_mode: wgpu::PresentMode::Fifo,
			desired_maximum_frame_latency: 2,
			alpha_mode: capabilities.alpha_modes[0],
			view_formats: Vec::new(),
			color_space: wgpu::SurfaceColorSpace::Auto,
		};
		// The page copying paints (each submitted, then waited for) as fast as it can.
		let stop = Arc::new(AtomicBool::new(false));
		let painter = std::thread::spawn({
			let (page, stop) = (page.clone(), stop.clone());
			move || {
				let (side, mut paints) = (64, 0u64);
				let paint = vec![0x80; (side * side * 4) as usize];
				let whole = [UiRegion { x: 0, y: 0, width: side, height: side }];
				while !stop.load(Ordering::Relaxed) {
					page.write_for_tests(side, side, &paint, &whole);
					paints += 1;
				}
				paints
			}
		});
		// Each configure a size other than the last's, so each makes a new swapchain.
		let mut configure = |gated: bool| {
			config.width = if config.width == size.width.max(1) { config.width / 2 + 1 } else { size.width.max(1) };
			let error = if gated {
				gpu.configure_surface(&surface, &config)
			} else {
				gpu.catching_errors(|| surface.configure(&gpu.device, &config)).1
			};
			error.map(|e| e.to_string())
		};
		let started = Instant::now();
		let (mut plain, mut plain_failed) = (0, None);
		while plain_failed.is_none() && plain < PLAIN_TRIES && started.elapsed() < PLAIN_TIME {
			plain += 1;
			plain_failed = configure(false);
		}
		let plain_time = started.elapsed();
		let started = Instant::now();
		let failures: Vec<String> = (0..CONFIGURES).filter_map(|_| configure(true)).collect();
		let gated_time = started.elapsed();
		stop.store(true, Ordering::Relaxed);
		let paints = painter.join().expect("the painter");
		// Left to the process's end, as `bb` leaves them: under gamescope's Vulkan layer
		// the surface's teardown destroys the window's own, and the window's crashes.
		std::mem::forget(surface);
		std::mem::forget(window);
		eprintln!(
			"surface race: a plain configure failed after {plain} ({plain_time:.1?}): {}; \
			{} of {CONFIGURES} gated ones failed ({gated_time:.1?}); the page copied {paints} paints",
			plain_failed.as_deref().unwrap_or("none did"),
			failures.len()
		);
		assert!(failures.is_empty(), "configures failed with the page submitting: {failures:?}");
		let failed = plain_failed.expect("no plain configure failed: the test didn't reach the race");
		assert!(failed.contains("come idle"), "a plain configure failed otherwise: {failed}");
	}
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
/// settings go to the run's own folder. Skips without the mods it needs; fails if the
/// install's are not the pinned corpus (`bb_engine::mod_corpus`).
#[test]
fn bb_starts_a_modded_campaign_at_once() {
	let data = Path::new(STEAM_DATA_DIR);
	let mods = match bb_engine::mod_corpus::installed(data) {
		Ok(mods) => mods,
		Err(bb_engine::mod_corpus::CorpusError::Absent(why)) => return bb_engine::harness::skip(why),
		Err(e) => panic!("{e}"),
	};
	let has = |prefix: &str| mods.iter().any(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(prefix)));
	if !has("mod_modern_hooks") || !has("mod_msu") {
		bb_engine::harness::skip("Modern Hooks or MSU not installed");
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
#[cfg_attr(not(feature = "ui"), ignore = "needs the ui feature: without the page, the game starts a campaign, whose world map draws every frame")]
fn the_game_steps_at_the_displays_rate_whichever_frames_are_presented() {
	assert!(cfg!(feature = "ui"), "needs the ui feature");
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

/// The mouse moving over a still screen presents a frame every refresh of the display,
/// the last frame again where nothing changed, and a still mouse presents none: on a
/// variable-refresh display the monitor refreshes only as frames are presented, and the
/// system's cursor moves only as it does. At the main menu with the
/// page (nothing there changes while idle) on a 180 Hz display, the mouse moved before
/// every frame for two seconds (`BB_TEST_MOUSE_MOVES`, a headless gamescope having no
/// mouse) near the top left corner, where nothing on the menu answers it.
#[test]
#[cfg_attr(not(feature = "ui"), ignore = "needs the ui feature: without the page, the game starts a campaign, whose world map draws every frame")]
fn the_moving_mouse_presents_every_refresh_and_a_still_one_none() {
	assert!(cfg!(feature = "ui"), "needs the ui feature");
	let refresh = 180u32;
	let dir = run_dir("mouse_refresh");
	let (settings, times) = (dir.join("settings.cfg"), dir.join("frame_times.txt"));
	let quit = r#"local menu = ::RootState.get("MainMenuState"); if (menu != null) menu.main_menu_module_onQuitPressed();"#;
	// Still for five seconds (the page loading, the menu showing, then nothing), the
	// mouse moving for two, still for three.
	let (moves, end) = ((refresh * 5, refresh * 7), refresh * 10);
	let end = end.to_string();
	let args = ["--settings", settings.to_str().unwrap(), "--at", &end, quit];
	let moving = format!("{}-{}", moves.0, moves.1);
	let env = [("BB_FRAME_TIMES", times.to_str().unwrap()), ("BB_TEST_MOUSE_MOVES", moving.as_str())];
	let Some(run) = run_bb_at(&dir, (1280, 720), Some(refresh), &args, &env) else { return };
	assert_eq!(run.status, "0", "{}", run.stderr);
	// Each frame: its number, its interval since the last, whether it was presented, and
	// whether that was the last frame again.
	let frames: Vec<(u32, f64, bool, bool)> = std::fs::read_to_string(&times)
		.expect("the frame times")
		.lines()
		.filter_map(|line| {
			let field = |name: &str| line.split(' ').find_map(|f| f.strip_prefix(name)).and_then(|v| v.parse::<f64>().ok());
			let frame = line.split(' ').next()?.parse().ok()?;
			Some((frame, field("interval=")?, field("present=").is_some(), field("re-presented#=").is_some()))
		})
		.collect();
	let within = |from: u32, to: u32| frames.iter().filter(move |f| (from..to).contains(&f.0));
	// The second before the moves, the moves (the first few frames left out: a move
	// reaches the frame it starts), and the seconds after them (a frame or so after the
	// last move left out).
	let still_before: Vec<_> = within(moves.0 - refresh, moves.0).collect();
	let moving: Vec<_> = within(moves.0 + 2, moves.1).collect();
	let still_after: Vec<_> = within(moves.1 + 4, refresh * 10 - 10).collect();
	let presented = |frames: &[&(u32, f64, bool, bool)]| frames.iter().filter(|f| f.2).count();
	let again = moving.iter().filter(|f| f.3).count();
	let rate = 1000.0 * presented(&moving) as f64 / moving.iter().map(|f| f.1).sum::<f64>();
	eprintln!(
		"{refresh} Hz: still before {} of {} presented; moving {} of {} ({again} the last again), {rate:.1} presents a second; \
		still after {} of {}",
		presented(&still_before),
		still_before.len(),
		presented(&moving),
		moving.len(),
		presented(&still_after),
		still_after.len()
	);
	assert!(moving.len() as u32 > refresh * 19 / 10, "{} frames while the mouse moved", moving.len());
	assert_eq!(presented(&moving), moving.len(), "every frame presented while the mouse moves");
	assert!(again * 2 > moving.len(), "mostly the last frame again ({again} of {}): the menu changed as the mouse moved", moving.len());
	let expected = f64::from(refresh);
	assert!((rate - expected).abs() < expected * 0.03, "{rate:.1} presents a second on a {refresh} Hz display");
	// A still menu presents nothing, but for the odd change of its own.
	assert!(presented(&still_before) * 50 < still_before.len(), "{} of {} presented before the moves", presented(&still_before), still_before.len());
	assert!(presented(&still_after) * 50 < still_after.len(), "{} of {} presented after the moves", presented(&still_after), still_after.len());
	assert!(still_after.iter().all(|f| !f.3), "the last frame presented again with the mouse still");
}

/// Where the GPU can't run the game with its UI, `bb` exits at once and says what is
/// missing: on stderr, in log.html, and in a message box (here a stand-in `zenity`
/// that writes down what it was asked to show). wgpu on GL stands for a GPU without a
/// Vulkan driver. Linux's message box (zenity) only.
#[cfg(all(feature = "ui", target_os = "linux"))]
#[test]
fn bb_says_what_it_lacks_on_a_gpu_without_vulkan() {
	let dir = run_dir("no_vulkan");
	let (bin, shown) = (dir.join("bin"), dir.join("dialog.txt"));
	std::fs::create_dir_all(&bin).unwrap();
	let zenity = bin.join("zenity");
	std::fs::write(&zenity, format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\n", shown.display())).unwrap();
	std::fs::set_permissions(&zenity, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
	let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default());
	let env = [("WGPU_BACKEND", "gl"), ("PATH", path.as_str()), ("BB_ERROR_DIALOG", "1")];
	let Some(run) = run_bb_with(&dir, (1280, 720), &[], &env) else { return };
	assert_eq!(run.status, "1", "{}", run.stderr);
	let what = "can't start on this computer: the GPU has no usable Vulkan driver";
	assert!(run.stderr.contains(what), "{}", run.stderr);
	assert!(run.stderr.contains("requirements.md"), "{}", run.stderr);
	let html = std::fs::read_to_string(dir.join("logs").join("log.html")).expect("log.html");
	assert!(html.contains(what), "{html}");
	let dialog = std::fs::read_to_string(&shown).expect("the message box was shown");
	assert!(dialog.contains("--error") && dialog.contains(what), "{dialog}");
}

/// A software GPU (lavapipe) is refused at once with a message, not left to Chromium.
#[cfg(feature = "ui")]
#[test]
fn bb_refuses_a_software_gpu_with_a_message() {
	let dir = run_dir("software_gpu");
	let lavapipe = "/usr/share/vulkan/icd.d/lvp_icd.json";
	if !Path::new(lavapipe).exists() {
		bb_engine::harness::skip(format!("no lavapipe ({lavapipe})"));
		return;
	}
	let env = [("VK_DRIVER_FILES", lavapipe), ("VK_ICD_FILENAMES", lavapipe), ("WGPU_BACKEND", "vulkan")];
	let Some(run) = run_bb_with(&dir, (1280, 720), &[], &env) else { return };
	assert_eq!(run.status, "1", "{}", run.stderr);
	assert!(run.stderr.contains("can't start on this computer: no hardware GPU was found, only the software renderer"), "{}", run.stderr);
}

/// Chromium's GPU compositing failing (here turned off with `--disable-gpu`): it paints
/// the page in memory instead, which can't be shown, and `bb` says so (at once, not
/// after the 10 s it waits for a first paint on the GPU).
#[cfg(feature = "ui")]
#[test]
fn bb_says_so_when_chromium_cant_paint_on_the_gpu() {
	let dir = run_dir("no_gpu_paints");
	let Some(run) = run_bb_with(&dir, (1280, 720), &[], &[("BB_UI_SWITCHES", "disable-gpu")]) else { return };
	assert_eq!(run.status, "1", "{}", run.stderr);
	assert!(
		run.stderr.contains("can't start on this computer: Chromium couldn't draw the UI on the GPU (Chromium's GPU compositing failed"),
		"{}",
		run.stderr
	);
}

/// An empty folder in the run's folder: where `BB_STEAM_DIR` points to hide this
/// machine's Steam install from `bb` (there's no Steam in it). The tests give `bb` an
/// empty `BB_DATA_DIR` too, and a settings file of their own (no folder chosen before).
#[cfg(any(feature = "ui", target_os = "linux"))]
fn no_steam(dir: &Path) -> String {
	let none = dir.join("no_steam");
	std::fs::create_dir_all(&none).expect("an empty folder");
	none.to_string_lossy().into_owned()
}

/// A stand-in for `tool` on `bb`'s `PATH` (in the run's folder `bin`) that writes its
/// arguments to `<tool>.args` there and prints `prints`: the message box's and the folder
/// dialog's tools, answered without a display. Returns the `PATH` to give `bb`.
#[cfg(target_os = "linux")]
fn fake_tool(dir: &Path, tool: &str, prints: &str) -> String {
	let bin = dir.join("bin");
	std::fs::create_dir_all(&bin).unwrap();
	let args = bin.join(format!("{tool}.args"));
	let script = format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\nprintf '%s\\n' '{prints}'\n", args.display());
	let path = bin.join(tool);
	std::fs::write(&path, script).unwrap();
	std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
	format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default())
}

/// The game's folder (above its data folder), for the folder dialog to answer.
#[cfg(all(feature = "ui", target_os = "linux"))]
fn game_folder() -> String {
	Path::new(STEAM_DATA_DIR).parent().expect("the game's folder").to_string_lossy().into_owned()
}

/// With no install to be found (no Steam, nothing chosen before), `bb` with the page says
/// so in a window of its own, with where it looked and how to name the folder in the
/// settings file by hand, and waits for the player.
#[cfg(feature = "ui")]
#[test]
fn bb_asks_for_the_game_folder_when_it_finds_none() {
	let dir = run_dir("not_found");
	let (settings, shot) = (dir.join("settings.cfg"), dir.join("shot.png"));
	let none = no_steam(&dir);
	// The page's text, through its console (its errors reach stderr).
	let report = "console.error('page says: ' + document.querySelector('h1').textContent + ' / ' + document.getElementById('bb-looked').textContent \
		+ ' / ' + document.getElementById('bb-settings-file').textContent + ' / ' + document.getElementById('bb-line').textContent.split('\\n')[0])";
	let args = ["--settings", settings.to_str().unwrap(), "--at-page", "5", report, "--shot", "10", shot.to_str().unwrap()];
	let Some(run) = run_bb_with(&dir, (1280, 720), &args, &[("BB_STEAM_DIR", &none), ("BB_DATA_DIR", "")]) else { return };
	assert_eq!(run.status, "0", "{}", run.stderr);
	assert!(run.stderr.contains("Battle Brothers wasn't found. Looked in:"), "{}", run.stderr);
	let page = format!(
		"page says: Battle Brothers wasn't found / Steam in {none}: not installed there / {} / install.data_dir = <the game's data folder>",
		settings.display()
	);
	assert!(run.stderr.contains(&page), "{page}:\n{}", run.stderr);
	assert!(shot.exists(), "the window's shot");
	let html = std::fs::read_to_string(dir.join("logs").join("log.html")).expect("log.html");
	assert!(html.contains("wasn't found"), "{html}");
}

/// The player's folder: one without the game is refused on the page, with why; the one
/// the folder dialog picks (its `win32` folder, where the original's exe is) is kept in
/// the settings, and `bb` starts again and plays it, at the main menu. The dialog is a
/// stand-in for zenity, Linux's.
#[cfg(all(feature = "ui", target_os = "linux"))]
#[test]
fn the_folder_the_player_picks_is_checked_kept_and_played() {
	let dir = run_dir("picked");
	let (settings, shot) = (dir.join("settings.cfg"), dir.join("shot.png"));
	let none = no_steam(&dir);
	let win32 = format!("{}/win32", game_folder());
	let path = fake_tool(&dir, "zenity", &win32);
	// Each step only on the not-found page: the started game runs them too, in its own.
	let on_page = |js: &str| format!("if (window.bbInstall) {{ {js} }}");
	let wrong = on_page(&format!(
		"document.getElementById('bb-path').value = {:?}; document.getElementById('bb-use').click();",
		dir.display().to_string()
	));
	let refused = on_page("console.error('refused: ' + document.getElementById('bb-error').textContent);");
	let browse = on_page("document.getElementById('bb-browse').click();");
	let report = r#"::logWarning("started at the " + (::RootState.get("MainMenuState") != null ? "menu" : "world"));"#;
	let args = [
		"--settings", settings.to_str().unwrap(),
		"--at-page", "5", &wrong,
		"--at-page", "10", &refused,
		"--at-page", "15", &browse,
		"--at", "30", report,
		"--shot", "40", shot.to_str().unwrap(),
	];
	let Some(run) = run_bb_with(&dir, (1280, 720), &args, &[("BB_STEAM_DIR", &none), ("BB_DATA_DIR", ""), ("PATH", &path)]) else { return };
	assert_eq!(run.status, "0", "{}", run.stderr);
	assert!(run.stderr.contains(&format!("refused: {} isn't Battle Brothers' folder", dir.display())), "{}", run.stderr);
	let asked = std::fs::read_to_string(dir.join("bin").join("zenity.args")).expect("the folder dialog was opened");
	assert!(asked.contains("--directory"), "{asked}");
	assert!(run.stderr.contains("install: the game's data: ") && run.stderr.contains("(from the folder chosen earlier)"), "{}", run.stderr);
	assert!(run.stderr.contains("started at the menu"), "{}", run.stderr);
	let kept = std::fs::read_to_string(&settings).expect("the settings");
	assert!(kept.contains(&format!("install.data_dir = {}", STEAM_DATA_DIR)), "{kept}");
}

/// Without the page (a developer's build) there's no window to ask in: `bb` ends with
/// where it looked and how to name the folder, in the message box too.
#[cfg(all(not(feature = "ui"), target_os = "linux"))]
#[test]
fn bb_without_the_ui_says_where_it_looked() {
	let dir = run_dir("not_found_plain");
	let none = no_steam(&dir);
	let path = fake_tool(&dir, "zenity", "");
	let env = [("BB_STEAM_DIR", none.as_str()), ("BB_DATA_DIR", ""), ("PATH", path.as_str()), ("BB_ERROR_DIALOG", "1")];
	let settings = dir.join("settings.cfg");
	let Some(run) = run_bb_with(&dir, (1280, 720), &["--settings", settings.to_str().unwrap()], &env) else { return };
	assert_eq!(run.status, "1", "{}", run.stderr);
	assert!(run.stderr.contains("Battle Brothers wasn't found. Looked in:"), "{}", run.stderr);
	let shown = std::fs::read_to_string(dir.join("bin").join("zenity.args")).expect("the message box was shown");
	assert!(shown.contains("--error") && shown.contains("Start bb with --data"), "{shown}");
	assert!(shown.contains(&format!("add this line to {}", settings.display())) && shown.contains("install.data_dir = "), "{shown}");
}

/// The command line's errors go to stderr (and the log) only, never a message box: a
/// player who double-clicks bb passes no arguments. Here `--data` naming a folder
/// without the game, and an unknown argument.
#[cfg(target_os = "linux")]
#[test]
fn a_command_line_error_is_not_shown_in_a_message_box() {
	let dir = run_dir("bad_data");
	let path = fake_tool(&dir, "zenity", "");
	let env = [("PATH", path.as_str()), ("BB_ERROR_DIALOG", "1")];
	let Some(run) = run_bb_with(&dir, (1280, 720), &["--data", dir.to_str().unwrap()], &env) else { return };
	assert_eq!(run.status, "1", "{}", run.stderr);
	let what = "--data names no Battle Brothers install";
	assert!(run.stderr.contains(what), "{}", run.stderr);
	let html = std::fs::read_to_string(dir.join("logs").join("log.html")).expect("log.html");
	assert!(html.contains(what), "{html}");
	let shown = dir.join("bin").join("zenity.args");
	assert!(!shown.exists(), "no message box: {}", std::fs::read_to_string(&shown).unwrap_or_default());
	let Some(run) = run_bb_with(&dir, (1280, 720), &["--no-such-argument"], &env) else { return };
	assert_eq!(run.status, "1", "{}", run.stderr);
	assert!(run.stderr.contains("unknown argument --no-such-argument"), "{}", run.stderr);
	assert!(!shown.exists(), "no message box: {}", std::fs::read_to_string(&shown).unwrap_or_default());
}

/// One bb at a time on a saves folder: while its folder is held (here by
/// the test, through the lock bb takes), a second bb on it says the game is already
/// running, in the message box too (a stand-in zenity), and ends before any window or
/// CEF; one on another folder plays meanwhile.
#[cfg(target_os = "linux")]
#[test]
fn a_second_bb_on_the_same_saves_is_refused() {
	let dir = run_dir("second_instance");
	let (saves, other, settings, shot) = (dir.join("saves"), dir.join("other_saves"), dir.join("settings.cfg"), dir.join("shot.png"));
	let _first = bb_app::saves::lock::SavesLock::take(&saves).unwrap().expect("the saves' lock");
	let path = fake_tool(&dir, "zenity", "");
	let env = [("PATH", path.as_str()), ("BB_ERROR_DIALOG", "1")];
	let Some(run) = run_bb_with(&dir, (1280, 720), &["--settings", settings.to_str().unwrap()], &env) else { return };
	assert_eq!(run.status, "1", "{}", run.stderr);
	let what = format!("Battle Brothers is already running, with its saves in {}.", saves.display());
	assert!(run.stderr.contains(&what), "{}", run.stderr);
	// CEF never started: it wrote no log.
	let cef = std::fs::metadata(dir.join("logs").join("cef.log")).map_or(0, |m| m.len());
	assert_eq!(cef, 0, "CEF started: {}", run.stderr);
	let shown = std::fs::read_to_string(dir.join("bin").join("zenity.args")).expect("the message box was shown");
	assert!(shown.contains("--error") && shown.contains(&what), "{shown}");
	let html = std::fs::read_to_string(dir.join("logs").join("log.html")).expect("log.html");
	assert!(html.contains("is already running"), "{html}");
	let args = ["--new-campaign", "--settings", settings.to_str().unwrap(), "--saves", other.to_str().unwrap(), "--shot", "3", shot.to_str().unwrap()];
	let Some(run) = run_bb(&dir, (1280, 720), &args) else { return };
	assert_eq!(run.status, "0", "{}", run.stderr);
	assert!(shot.exists(), "the other folder's game drew");
}

/// A Steam library of the run's own (`lib/steamapps`) with the game in it: the install's
/// archives and `gfx` linked into a data folder of its own (so it's this library's,
/// not the real one's), and Steam's manifest for it. Returns the data folder and the
/// library's `steamapps`. Its mods aren't linked: the run is vanilla.
#[cfg(target_os = "linux")]
fn fake_steam_library(dir: &Path) -> (PathBuf, PathBuf) {
	let steamapps = dir.join("lib").join("steamapps");
	let data = steamapps.join("common").join("Battle Brothers").join("data");
	std::fs::create_dir_all(&data).expect("the fake library");
	for entry in std::fs::read_dir(STEAM_DATA_DIR).expect("the install's data folder").flatten() {
		let name = entry.file_name();
		let text = name.to_string_lossy();
		if (text.starts_with("data_") && text.ends_with(".dat")) || text == "gfx" {
			std::os::unix::fs::symlink(entry.path(), data.join(&name)).expect("a link to the install's file");
		}
	}
	let manifest = "\"AppState\"\n{\n\t\"appid\"\t\t\"365360\"\n\t\"installdir\"\t\t\"Battle Brothers\"\n}\n";
	std::fs::write(steamapps.join("appmanifest_365360.acf"), manifest).expect("the fake manifest");
	(data, steamapps)
}

/// With nothing naming a folder for the saves, `bb` keeps them in the original game's
/// savegames folder, the player's own: on Linux, in the Proton prefix of the Steam
/// install played (here a fake library's, never the real one). A campaign saved lands
/// there. Without the prefix (the original never ran there), bb's own folder.
#[cfg(target_os = "linux")]
#[test]
fn the_saves_go_to_the_original_games_folder_by_default() {
	let dir = run_dir("default_saves");
	let (data, steamapps) = fake_steam_library(&dir);
	let user = steamapps.join("compatdata/365360/pfx/drive_c/users/steamuser");
	std::fs::create_dir_all(&user).expect("the fake prefix");
	let savegames = user.join("Documents/Battle Brothers/savegames");
	let settings = dir.join("settings.cfg");
	let save = r#"local up = ("State" in ::World) && ::World.State != null && ::World.State.getPlayer() != null && !::World.State.isInLoadingScreen();
if (up && !("__saved" in getroottable())) { ::__saved <- true; ::World.State.saveCampaign("bb_probe_default_saves"); ::logWarning("saved the campaign"); ::World.State.m.MenuStack.popAll(true); ::World.State.exitGame(); }"#;
	let quit = r#"local menu = ::RootState.get("MainMenuState"); if (("__saved" in getroottable()) && ::RootState.get("WorldState") == null && menu != null) menu.main_menu_module_onQuitPressed();"#;
	let frames: Vec<String> = (1..=400).map(|i| (i * 30).to_string()).collect();
	let mut args = vec!["--data", data.to_str().unwrap(), "--new-campaign", "--settings", settings.to_str().unwrap()];
	for frame in &frames {
		args.extend(["--at", frame.as_str(), save, "--at", frame.as_str(), quit]);
	}
	// `BB_SAVES_DIR` empty: as if not set (the harness sets it to the run's folder).
	let xdg = dir.join("xdg").to_string_lossy().into_owned();
	let env = [("BB_SAVES_DIR", ""), ("XDG_DATA_HOME", xdg.as_str())];
	let Some(run) = run_bb_with(&dir, (1280, 720), &args, &env) else { return };
	assert_eq!(run.status, "0", "{}", run.stderr);
	let line = format!("saves: {} (from the original game's savegames folder)", savegames.display());
	assert!(run.stderr.contains(&line), "{line}:\n{}", run.stderr);
	assert!(run.stderr.contains("saved the campaign"), "{}", run.stderr);
	assert!(savegames.join("bb_probe_default_saves.sav").is_file(), "the save in the prefix's savegames folder");

	// No prefix: bb's own folder in the user's data folder (here the run's).
	std::fs::remove_dir_all(steamapps.join("compatdata")).expect("the fake prefix removed");
	let shot = dir.join("shot.png");
	let args = ["--data", data.to_str().unwrap(), "--settings", settings.to_str().unwrap(), "--shot", "20", shot.to_str().unwrap()];
	let Some(run) = run_bb_with(&dir, (1280, 720), &args, &env) else { return };
	assert_eq!(run.status, "0", "{}", run.stderr);
	let own = Path::new(&xdg).join("battle-brothers-modern/savegames");
	let line = format!("saves: {} (from bb's own folder: no Proton prefix", own.display());
	assert!(run.stderr.contains(&line), "{line}:\n{}", run.stderr);
	assert!(!steamapps.join("compatdata").exists(), "no prefix made");
}
