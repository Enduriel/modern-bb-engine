//! What differs between the systems the window runs on, one small function each.

use winit::event_loop::ActiveEventLoop;

/// Whether a window can take the monitor in a mode of its own (exclusive fullscreen).
/// Wayland has no such thing (winit ignores it there).
#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
pub fn exclusive_fullscreen(event_loop: &ActiveEventLoop) -> bool {
	use winit::platform::wayland::ActiveEventLoopExtWayland;
	!event_loop.is_wayland()
}

#[cfg(not(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android")))))]
pub fn exclusive_fullscreen(_event_loop: &ActiveEventLoop) -> bool {
	true
}

/// The smallest buffer the allocator always takes from the system and gives back when
/// freed: the size of the smallest atlas (2048×1024 RGBA).
#[cfg(all(target_os = "linux", target_env = "gnu"))]
const SYSTEM_BUFFER: i32 = 8 << 20;

/// How many arenas (free lists) the C allocator keeps for the process's threads.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
const ARENAS: i32 = 2;

/// Tune the C allocator for the game's work, before any thread starts (glibc):
/// - Atlases decode on worker threads into buffers of 8 to 64 MiB, which the game's
///   thread frees after uploading them. glibc raises its threshold for taking a buffer
///   straight from the system after each such free (up to 32 MiB), so later buffers
///   come from the workers' arenas instead, and stay held there once freed: on a fresh
///   world (measured 2026-10-14) 393 MB resident, 304 MB with the
///   threshold fixed at 8 MiB (`M_MMAP_THRESHOLD`; the frame cost unchanged). Buffers
///   below it (a frame's vertices, the scripts') stay with the allocator.
/// - glibc gives each thread an arena of its own, up to 8 per core, and memory freed in
///   one stays there: the decode workers', the sound loaders', and those of the
///   browser's threads running in the game's process. Two arenas for all of them
///   (`M_ARENA_MAX`), and for the browser's own processes too (`MALLOC_ARENA_MAX`, read
///   as each starts): with the UI, a fresh world 650 -> 615 MB (PSS, every process),
///   and after three battles 1425 -> 837, the frame cost unchanged; four arenas
///   saved about half as much.
/// Other systems' allocators give buffers this large back by themselves.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
pub fn tune_allocator() {
	/// glibc's `mallopt` parameters (`malloc.h`).
	const M_MMAP_THRESHOLD: i32 = -3;
	const M_ARENA_MAX: i32 = -8;
	unsafe extern "C" {
		fn mallopt(param: i32, value: i32) -> i32;
	}
	// SAFETY: `mallopt` only sets the allocator's parameters, and setting the variable
	// is sound with no other thread running: both happen before any starts.
	unsafe {
		if mallopt(M_MMAP_THRESHOLD, SYSTEM_BUFFER) != 1 || mallopt(M_ARENA_MAX, ARENAS) != 1 {
			eprintln!("allocator: could not set its parameters");
		}
		std::env::set_var("MALLOC_ARENA_MAX", ARENAS.to_string());
	}
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
pub fn tune_allocator() {}

/// Open `url` in the system's browser, as the original does for the page's `openURL`:
/// start the system's launcher for it and return without waiting. A thread waits for
/// the launcher instead (no zombie is left); its output goes nowhere, not to the game's
/// terminal. The caller checks the URL first (`bb_ui::web_url::to_open`); it reaches the
/// launcher as one argument, never through a shell.
pub fn open_url(url: &str) -> Result<(), String> {
	use std::process::Stdio;
	let mut command = url_launcher(url).ok_or("no browser launcher on this system")?;
	let program = command.get_program().to_string_lossy().into_owned();
	let mut child = command
		.stdin(Stdio::null())
		.stdout(Stdio::null())
		.stderr(Stdio::null())
		.spawn()
		.map_err(|e| format!("{program}: {e}"))?;
	std::thread::Builder::new()
		.name("url-launcher".into())
		.spawn(move || child.wait())
		.map_err(|e| format!("waiting for {program}: {e}"))?;
	Ok(())
}

/// The command that opens `url` in the system's browser: the desktop's opener
/// (freedesktop's `xdg-open`).
#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
fn url_launcher(url: &str) -> Option<std::process::Command> {
	let mut command = std::process::Command::new("xdg-open");
	command.arg(url);
	Some(command)
}

/// The command that opens `url` in the system's browser: `open`, by its full path.
#[cfg(target_os = "macos")]
fn url_launcher(url: &str) -> Option<std::process::Command> {
	let mut command = std::process::Command::new("/usr/bin/open");
	command.arg(url);
	Some(command)
}

/// The command that opens `url` in the system's browser: the shell's URL handler,
/// through `rundll32` (no `cmd`, so nothing in the URL is a command's syntax). A
/// checked URL has no spaces or quotes, so it reaches the handler as it is.
#[cfg(windows)]
fn url_launcher(url: &str) -> Option<std::process::Command> {
	let mut command = std::process::Command::new("rundll32.exe");
	command.args(["url.dll,FileProtocolHandler", url]);
	Some(command)
}

#[cfg(any(target_os = "ios", target_os = "android", not(any(unix, windows))))]
fn url_launcher(_url: &str) -> Option<std::process::Command> {
	None
}

#[cfg(test)]
mod tests {
	/// The launcher gets the URL as its one argument, as it is (never through a shell).
	#[test]
	fn the_launcher_gets_the_url_alone() {
		let url = "https://www.nexusmods.com/battlebrothers/mods/765?tab=files&x=$(id)";
		let command = super::url_launcher(url).expect("a launcher on this system");
		let args: Vec<_> = command.get_args().map(|a| a.to_str().unwrap()).collect();
		let program = command.get_program().to_str().unwrap();
		#[cfg(all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android"))))]
		assert_eq!((program, args.as_slice()), ("xdg-open", [url].as_slice()));
		#[cfg(target_os = "macos")]
		assert_eq!((program, args.as_slice()), ("/usr/bin/open", [url].as_slice()));
		#[cfg(windows)]
		assert_eq!(
			(program, args.as_slice()),
			("rundll32.exe", ["url.dll,FileProtocolHandler", url].as_slice())
		);
	}
}
