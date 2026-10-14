//! `bb` exits cleanly. Under gamescope (headless, with its Vulkan layer on, as for every
//! game it runs) the window's teardown once crashed in libwayland after the surface
//! drawn to it was destroyed (see `Running::close`). The game runs muted in a headless
//! gamescope, so nothing shows or sounds. Skips cleanly without gamescope or the Steam
//! install.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use bb_engine::harness::STEAM_DATA_DIR;

#[test]
fn bb_exits_cleanly_under_gamescope() {
	if !Path::new(STEAM_DATA_DIR).exists() {
		eprintln!("skipping: {STEAM_DATA_DIR} not present");
		return;
	}
	let gamescope = Command::new("gamescope").arg("--help").stdout(Stdio::null()).stderr(Stdio::null()).status();
	if gamescope.is_err() {
		eprintln!("skipping: no gamescope");
		return;
	}
	let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("bb_exit");
	std::fs::create_dir_all(&dir).expect("a directory for the run");
	let (shot, status) = (dir.join("shot.png"), dir.join("status"));
	let _ = std::fs::remove_file(&shot);
	let _ = std::fs::remove_file(&status);

	// gamescope doesn't pass its child's status on: the shell writes it. Its own Wayland
	// display (`--expose-wayland`) stands in for the desktop's, so the window opens there.
	let mut child = Command::new("gamescope")
		.args(["--backend", "headless", "--expose-wayland", "-W", "640", "-H", "360", "--"])
		.args(["sh", "-c", r#""$0" --mute --shot 3 "$1"; echo $? > "$2""#])
		.arg(env!("CARGO_BIN_EXE_bb"))
		.arg(&shot)
		.arg(&status)
		.stdout(Stdio::null())
		.stderr(Stdio::null())
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
	let code = std::fs::read_to_string(&status).expect("bb's exit status");
	assert_eq!(code.trim(), "0", "bb's exit status (139: the crash at exit)");
	assert!(shot.exists(), "the shot taken before the exit");
}
