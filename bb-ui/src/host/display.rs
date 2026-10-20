//! The display connection Chromium paints the page through.
//!
//! The page is painted on the GPU and handed over as GPU buffers (see
//! [`paint_sink`](super::paint_sink)). On Linux, Chromium only allocates those through a
//! windowing system's platform (ozone X11 or Wayland): its headless platform has no
//! native buffers and delivers no paints at all. So the page needs a display connection,
//! though it never opens a window: the game's own (its window's), or, for the tests and
//! tools, a headless gamescope's ([`with_display`]).

use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};

/// Which windowing system Chromium connects to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Display {
	X11,
	Wayland,
}

impl Display {
	/// The display connection this process has: Wayland's if `WAYLAND_DISPLAY` is set
	/// (as winit picks it), else X11's if `DISPLAY` is; `BB_UI_OZONE=x11|wayland`
	/// chooses. None: an error saying what to do.
	pub fn from_env() -> Result<Display, String> {
		let set = |name: &str| std::env::var_os(name).is_some_and(|v| !v.is_empty());
		match std::env::var("BB_UI_OZONE").ok().as_deref() {
			Some("x11") => return Ok(Display::X11),
			Some("wayland") => return Ok(Display::Wayland),
			Some(other) => return Err(format!("BB_UI_OZONE={other}: say x11 or wayland")),
			None => {}
		}
		if set("WAYLAND_DISPLAY") {
			Ok(Display::Wayland)
		} else if set("DISPLAY") {
			Ok(Display::X11)
		} else {
			Err("no display connection (neither WAYLAND_DISPLAY nor DISPLAY is set): the UI page is \
				painted on the GPU, which Chromium does through X11 or Wayland. Run the game in a \
				desktop session, or headless inside `gamescope --backend headless -- ...`"
				.into())
		}
	}

	/// Chromium's `--ozone-platform` for it.
	pub(super) fn ozone(self) -> &'static str {
		match self {
			Display::X11 => "x11",
			Display::Wayland => "wayland",
		}
	}
}

/// Says where a wrapped process runs: unset, [`with_display`] starts a headless
/// gamescope; `gamescope` inside it; `current` uses this session's display connection.
const MODE: &str = "BB_UI_DISPLAY";
/// Where the wrapped process writes its exit status (gamescope doesn't pass it on).
const STATUS: &str = "BB_UI_DISPLAY_STATUS";

/// Run `run` (a test's or tool's `main`, after [`run_subprocess`](super::run_subprocess))
/// with a display connection for the page, never the player's: this program runs again
/// inside a headless gamescope (nothing shows on any screen), and its exit status comes
/// back out. `BB_UI_DISPLAY=current` runs it on the session's own display connection
/// instead. Without gamescope it fails, saying so.
pub fn with_display(run: impl FnOnce() -> ExitCode) -> ExitCode {
	match std::env::var(MODE).ok().as_deref() {
		Some("current" | "gamescope") => {
			let code = run();
			if let Some(path) = std::env::var_os(STATUS) {
				let status = if code == ExitCode::SUCCESS { "0" } else { "1" };
				if let Err(e) = std::fs::write(&path, status) {
					eprintln!("bb-ui: could not pass the exit status on ({}): {e}", PathBuf::from(path).display());
				}
			}
			code
		}
		Some(other) => {
			eprintln!("bb-ui: {MODE}={other}: say `current` (this session's display) or leave it unset (a headless gamescope)");
			ExitCode::FAILURE
		}
		None => match in_gamescope() {
			Ok(code) => code,
			Err(e) => {
				eprintln!("bb-ui: {e}");
				ExitCode::FAILURE
			}
		},
	}
}

/// Lines gamescope (and its Vulkan layer, in our process) prints about itself.
fn gamescope_noise(line: &str) -> bool {
	const PREFIXES: &[&str] = &[
		"[gamescope]",
		"[Gamescope WSI]",
		"wlserver:",
		"xwm:",
		"vulkan:",
		"No CAP_SYS_NICE",
		"Performance will be affected",
		"ATTENTION: default value of option vk_xwayland_wait_ready",
		"(EE) failed to read Wayland events",
		"WARNING: radv is not a conformant Vulkan implementation",
		"  pApplicationName:",
		"  applicationVersion:",
		"  pEngineName:",
		"  engineVersion:",
		"  apiVersion:",
	];
	PREFIXES.iter().any(|p| line.starts_with(p))
}

/// Pass `from`'s lines on to `to`, without gamescope's own.
fn forward(from: impl Read + Send + 'static, mut to: impl Write + Send + 'static) -> std::thread::JoinHandle<()> {
	std::thread::spawn(move || {
		for line in BufReader::new(from).lines().map_while(Result::ok) {
			if !gamescope_noise(&line) {
				let _ = writeln!(to, "{line}");
			}
		}
	})
}

/// Run this program again, with its arguments, inside a headless gamescope.
fn in_gamescope() -> Result<ExitCode, String> {
	let exe = std::env::current_exe().map_err(|e| format!("this program's path: {e}"))?;
	let status = std::env::temp_dir().join(format!("bb-ui-display-{}.status", std::process::id()));
	let _ = std::fs::remove_file(&status);
	let mut child = Command::new("gamescope")
		.args(["--backend", "headless", "-W", "1280", "-H", "720", "--"])
		.arg(&exe)
		.args(std::env::args_os().skip(1))
		// Never the session's display: the page's connection is gamescope's.
		.env_remove("DISPLAY")
		.env_remove("WAYLAND_DISPLAY")
		.env(MODE, "gamescope")
		.env(STATUS, &status)
		.stdin(Stdio::null())
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.spawn()
		.map_err(|e| {
			format!(
				"could not start gamescope ({e}): the UI page needs a display connection, and the tests \
				make one with `gamescope --backend headless`. Install gamescope, or run with \
				{MODE}=current to use this session's display"
			)
		})?;
	let out = forward(child.stdout.take().expect("piped"), std::io::stdout());
	let err = forward(child.stderr.take().expect("piped"), std::io::stderr());
	let waited = child.wait().map_err(|e| format!("waiting for gamescope: {e}"))?;
	let _ = (out.join(), err.join());
	let passed = std::fs::read_to_string(&status);
	let _ = std::fs::remove_file(&status);
	match passed.as_deref().map(str::trim) {
		Ok("0") => Ok(ExitCode::SUCCESS),
		Ok(_) => Ok(ExitCode::FAILURE),
		Err(_) => Err(format!("the run inside gamescope ended without an exit status (gamescope: {waited})")),
	}
}
