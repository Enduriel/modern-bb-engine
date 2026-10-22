//! The system's folder dialog for the not-found window, each system's own way:
//!
//! - Windows: Chromium's (the page's browser, CEF's `RunFileDialog`: the system's
//!   folder picker); its answer comes as the host's `Event::FolderChosen`.
//! - Linux: `zenity`, else `kdialog` (GNOME's and KDE's dialog tools; Steam's runtime and
//!   most desktops have one), as the message box does (`requirements::dialog`). CEF's
//!   own can't open here: it refuses with the multi-threaded message loop bb runs it
//!   with ("Default dialog implementation is not available", CEF's
//!   `file_dialog_manager.cc`, seen 2026-10-22). Neither tool: the dialog is
//!   unavailable, and the page asks for the folder typed.
//! - macOS: AppleScript's `choose folder` (`osascript`), as the message box uses it.
//!   CEF's would want a window of its own to hang the panel on, which the page (drawn
//!   offscreen) hasn't.
//!
//! The tools run on a thread of their own, so the window goes on drawing meanwhile.

use std::path::{Path, PathBuf};
use std::sync::mpsc;

use bb_ui::host::UiHost;

/// How the dialog closed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
	Folder(PathBuf),
	Cancelled,
	/// No dialog could open on this system.
	Unavailable,
}

/// The dialog, while it's open.
#[derive(Default)]
pub struct FolderDialog {
	/// The tool's answer, to come (Linux, macOS).
	answer: Option<mpsc::Receiver<Answer>>,
	/// Whether Chromium's is open (Windows).
	in_browser: bool,
}

impl FolderDialog {
	pub fn is_open(&self) -> bool {
		self.answer.is_some() || self.in_browser
	}

	/// Open it, titled `title`, at `start` if given (nothing if it's open already).
	pub fn open(&mut self, host: &UiHost, title: &str, start: Option<&Path>) {
		if self.is_open() {
			return;
		}
		if cfg!(windows) {
			self.in_browser = true;
			host.choose_folder(title, start);
			return;
		}
		let (tx, rx) = mpsc::channel();
		let (title, start) = (title.to_string(), start.map(Path::to_path_buf));
		let spawned = std::thread::Builder::new()
			.name("folder dialog".into())
			.spawn(move || {
				let _ = tx.send(run_tool(&title, start.as_deref()));
			});
		match spawned {
			Ok(_) => self.answer = Some(rx),
			Err(e) => {
				eprintln!("install: no folder dialog: {e}");
				let (tx, rx) = mpsc::channel();
				let _ = tx.send(Answer::Unavailable);
				self.answer = Some(rx);
			}
		}
	}

	/// Its answer, once it closed.
	pub fn poll(&mut self) -> Option<Answer> {
		let answer = self.answer.as_ref()?.try_recv();
		match answer {
			Ok(answer) => {
				self.answer = None;
				Some(answer)
			}
			Err(mpsc::TryRecvError::Empty) => None,
			Err(mpsc::TryRecvError::Disconnected) => {
				self.answer = None;
				Some(Answer::Unavailable)
			}
		}
	}

	/// Chromium's dialog closed (Windows): its folder, or none.
	pub fn browser_answered(&mut self, folder: Option<PathBuf>) -> Answer {
		self.in_browser = false;
		folder.map_or(Answer::Cancelled, Answer::Folder)
	}
}

/// Run the system's dialog tool and wait for its answer.
#[cfg(all(unix, not(target_os = "macos")))]
fn run_tool(title: &str, start: Option<&Path>) -> Answer {
	let start = start.map(|p| p.to_string_lossy().into_owned());
	// zenity wants a trailing slash to open inside a folder rather than at it.
	let zenity_start = start.as_ref().map(|s| format!("--filename={}/", s.trim_end_matches('/')));
	let tools: [(&str, Vec<String>); 2] = [
		(
			"zenity",
			["--file-selection".to_string(), "--directory".into(), format!("--title={title}")]
				.into_iter()
				.chain(zenity_start)
				.collect(),
		),
		(
			"kdialog",
			["--title".to_string(), title.into(), "--getexistingdirectory".into()]
				.into_iter()
				.chain(start.clone())
				.collect(),
		),
	];
	for (tool, args) in tools {
		match tool_answer(std::process::Command::new(tool).args(args)) {
			Some(answer) => return answer,
			None => continue,
		}
	}
	Answer::Unavailable
}

#[cfg(target_os = "macos")]
fn run_tool(title: &str, start: Option<&Path>) -> Answer {
	// AppleScript string literals: backslashes and quotes escaped.
	let quote = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
	let mut script = format!("POSIX path of (choose folder with prompt {}", quote(title));
	if let Some(start) = start {
		script.push_str(&format!(" default location (POSIX file {})", quote(&start.to_string_lossy())));
	}
	script.push(')');
	tool_answer(std::process::Command::new("osascript").args(["-e", &script])).unwrap_or(Answer::Unavailable)
}

#[cfg(not(unix))]
fn run_tool(_title: &str, _start: Option<&Path>) -> Answer {
	Answer::Unavailable
}

/// The folder a dialog tool printed, or `Cancelled` if it ended without one; `None` if
/// it couldn't run at all.
#[cfg(unix)]
fn tool_answer(command: &mut std::process::Command) -> Option<Answer> {
	let output = command.stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).output().ok()?;
	let printed = String::from_utf8_lossy(&output.stdout);
	let folder = printed.trim_end_matches(['\n', '\r']);
	Some(if output.status.success() && !folder.is_empty() { Answer::Folder(PathBuf::from(folder)) } else { Answer::Cancelled })
}

#[cfg(all(test, unix, not(target_os = "macos")))]
mod tests {
	use super::*;

	#[test]
	fn a_tools_printed_folder_is_the_answer() {
		let mut printed = std::process::Command::new("sh");
		printed.args(["-c", "printf '/games/Battle Brothers\\n'"]);
		assert_eq!(tool_answer(&mut printed), Some(Answer::Folder(PathBuf::from("/games/Battle Brothers"))));
		let mut cancelled = std::process::Command::new("sh");
		cancelled.args(["-c", "exit 1"]);
		assert_eq!(tool_answer(&mut cancelled), Some(Answer::Cancelled));
		assert_eq!(tool_answer(&mut std::process::Command::new("/nonexistent/zenity")), None);
	}
}
