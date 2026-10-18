//! `bb`'s window, run in a headless gamescope (muted, so nothing shows or sounds): it
//! exits cleanly, and the scripts' video mode is put on it as they set it. Skips
//! cleanly without gamescope or the Steam install, or when gamescope can't start (no
//! display, no runtime folder). Each run waits its turn behind the probes' games
//! ([`bb_engine::game_queue`]).

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
		.args(args)
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

/// The window's teardown once crashed in libwayland under gamescope's Vulkan layer,
/// after the surface drawn to it was destroyed (see `Running::close`).
#[test]
fn bb_exits_cleanly() {
	let dir = run_dir("exit");
	let (settings, shot) = (dir.join("settings.cfg"), dir.join("shot.png"));
	let args = ["--settings", settings.to_str().unwrap(), "--shot", "3", shot.to_str().unwrap()];
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
	let mut args = vec!["--settings", settings.to_str().unwrap()];
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
	let args = ["--settings", settings.to_str().unwrap(), "--shot", "300", shot.to_str().unwrap()];
	let Some(run) = run_bb(&dir, (1920, 1080), &args) else { return };
	assert_eq!(run.status, "0", "{}", run.stderr);
	assert!(run.stderr.contains("video: 1920x1080 Borderless"), "{}", run.stderr);
	assert_eq!(png_size(&shot), (1920, 1080), "borderless over the monitor");
}
