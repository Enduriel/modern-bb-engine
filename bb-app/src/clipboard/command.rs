//! The clipboard through the system's clipboard programs: `pbcopy`/`pbpaste` on macOS;
//! on Linux and the BSDs `wl-copy`/`wl-paste` on Wayland, else `xclip` or `xsel` on X11.
//! Each run gets the text as its input or gives it as its output, never through a
//! shell, and is stopped if it takes longer than [`TIMEOUT`] (an X11 selection's owner
//! that never answers).

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// How long a clipboard program may take.
const TIMEOUT: Duration = Duration::from_secs(1);

/// The programs that read and write the clipboard, and their arguments.
pub(super) struct Programs {
	name: &'static str,
	read: (PathBuf, &'static [&'static str]),
	write: (PathBuf, &'static [&'static str]),
}

/// A pair of programs: (name, reader, its arguments, writer, its arguments).
type Candidate = (&'static str, &'static str, &'static [&'static str], &'static str, &'static [&'static str]);

/// wl-clipboard. `--type text` picks the selection's text in whichever text type it has.
const WAYLAND: Candidate = ("wl-clipboard", "wl-paste", &["--no-newline", "--type", "text"], "wl-copy", &["--type", "text/plain;charset=utf-8"]);
const XCLIP: Candidate = ("xclip", "xclip", &["-selection", "clipboard", "-out"], "xclip", &["-selection", "clipboard", "-in"]);
const XSEL: Candidate = ("xsel", "xsel", &["--clipboard", "--output"], "xsel", &["--clipboard", "--input"]);

impl Programs {
	/// The programs for the display the game runs on, if they are installed.
	#[cfg(not(target_os = "macos"))]
	pub(super) fn find() -> Option<Programs> {
		let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some_and(|d| !d.is_empty());
		let x11 = std::env::var_os("DISPLAY").is_some_and(|d| !d.is_empty());
		let mut candidates = Vec::new();
		if wayland {
			candidates.push(WAYLAND);
		}
		if x11 {
			candidates.extend([XCLIP, XSEL]);
		}
		candidates.into_iter().find_map(|(name, read, read_args, write, write_args)| {
			Some(Programs { name, read: (on_path(read)?, read_args), write: (on_path(write)?, write_args) })
		})
	}

	/// `pbpaste` and `pbcopy`, by their full paths.
	#[cfg(target_os = "macos")]
	pub(super) fn find() -> Option<Programs> {
		Some(Programs {
			name: "pbcopy",
			read: (PathBuf::from("/usr/bin/pbpaste"), &[]),
			write: (PathBuf::from("/usr/bin/pbcopy"), &[]),
		})
	}

	pub(super) fn name(&self) -> &'static str {
		self.name
	}

	/// The clipboard's text; `Ok(None)` for none, `Err` if the program failed (no
	/// display, nothing on the clipboard to read).
	pub(super) fn read(&self) -> Result<Option<String>, ()> {
		let mut child = command(&self.read.0, self.read.1)
			.stdin(Stdio::null())
			.stdout(Stdio::piped())
			.stderr(Stdio::null())
			.spawn()
			.map_err(|_| ())?;
		let mut stdout = child.stdout.take().ok_or(())?;
		// Read on another thread, so a long text can't fill the pipe while we wait.
		let reader = std::thread::spawn(move || {
			let mut bytes = Vec::new();
			stdout.read_to_end(&mut bytes).map(|_| bytes)
		});
		let finished = wait(&mut child);
		let bytes = reader.join().map_err(|_| ())?.map_err(|_| ())?;
		if !finished? {
			return Err(());
		}
		Ok(Some(String::from_utf8_lossy(&bytes).into_owned()).filter(|t| !t.is_empty()))
	}

	/// Put `text` on the clipboard.
	pub(super) fn write(&self, text: &str) -> Result<(), ()> {
		let mut child = command(&self.write.0, self.write.1)
			.stdin(Stdio::piped())
			.stdout(Stdio::null())
			.stderr(Stdio::null())
			.spawn()
			.map_err(|_| ())?;
		let written = child.stdin.take().ok_or(()).and_then(|mut stdin| stdin.write_all(text.as_bytes()).map_err(|_| ()));
		// The input closes here, so the program can finish (the X11 and Wayland ones go
		// on in the background, serving the selection).
		let finished = wait(&mut child)?;
		written?;
		finished.then_some(()).ok_or(())
	}
}

fn command(program: &Path, args: &[&str]) -> Command {
	let mut command = Command::new(program);
	command.args(args);
	// pbcopy and pbpaste take and give text in the locale's encoding.
	#[cfg(target_os = "macos")]
	command.env("LANG", "en_US.UTF-8");
	command
}

/// Wait for `child` up to [`TIMEOUT`], stopping it if it runs over: whether it
/// succeeded.
fn wait(child: &mut Child) -> Result<bool, ()> {
	let deadline = Instant::now() + TIMEOUT;
	loop {
		match child.try_wait() {
			Ok(Some(status)) => return Ok(status.success()),
			Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(2)),
			Ok(None) => {
				let _ = child.kill();
				let _ = child.wait();
				return Ok(false);
			}
			Err(_) => return Err(()),
		}
	}
}

/// `program`'s path, if it is on the `PATH`.
#[cfg(not(target_os = "macos"))]
fn on_path(program: &str) -> Option<PathBuf> {
	let path = std::env::var_os("PATH")?;
	std::env::split_paths(&path).map(|dir| dir.join(program)).find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
	use super::Programs;

	/// Copies to and pastes from the display's real clipboard, so only when asked
	/// (`BB_CLIPBOARD_TEST=1`), inside a display of its own (e.g. a headless gamescope):
	/// it would replace the player's clipboard otherwise.
	#[test]
	fn text_goes_round_the_systems_clipboard() {
		if std::env::var_os("BB_CLIPBOARD_TEST").is_none_or(|v| v != "1") {
			eprintln!("skipping: set BB_CLIPBOARD_TEST=1 inside a display of its own");
			return;
		}
		let programs = Programs::find().expect("a clipboard program for this display");
		let text = "Ünïcode \"build\" code\nwith 2 lines | $(id)";
		programs.write(text).expect("written");
		assert_eq!(programs.read(), Ok(Some(text.to_string())), "read back with {}", programs.name());
		let long = "x".repeat(300_000);
		programs.write(&long).expect("written");
		assert_eq!(programs.read().map(|t| t.map(|t| t.len())), Ok(Some(long.len())));
	}
}
