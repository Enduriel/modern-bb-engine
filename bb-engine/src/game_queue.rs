//! A place in the queue for the machine's headless game display, shared with the
//! probe runner that launches the original game (`/tmp/bb-probe-queue`). Probes
//! run the original in a headless gamescope, and two gamescopes at once can hang one's teardown, so tests that
//! run gamescope take a [`Ticket`] and wait their turn behind the probes, as the probes
//! do behind each other.
//!
//! The runner's semantics: a ticket is a file named `<nanoseconds, 20 digits>-<pid>`;
//! the first live one (its process still running) holds the display until its file
//! goes; tickets of dead processes are dropped.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// The runner's queue.
const QUEUE: &str = "/tmp/bb-probe-queue";

/// A place in the queue; dropping it gives the display up, and so does the process's end
/// (a ticket kept in a static stays until then).
pub struct Ticket(PathBuf);

impl Ticket {
	/// Queue up and wait until first in line, for up to `max_wait`.
	pub fn wait(max_wait: Duration) -> Result<Ticket, String> {
		let dir = Path::new(QUEUE);
		fs::create_dir_all(dir).map_err(|e| format!("{QUEUE}: {e}"))?;
		let ns = SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.map_err(|e| e.to_string())?
			.as_nanos();
		let path = dir.join(format!("{ns:020}-{}", std::process::id()));
		fs::File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?;
		let ticket = Ticket(path);
		let deadline = Instant::now() + max_wait;
		while live_tickets(dir).first() != Some(&ticket.0) {
			if Instant::now() >= deadline {
				return Err(format!(
					"the game display stayed busy for {max_wait:?} ({QUEUE})"
				));
			}
			// A quarter of a second: a turn handed on is taken at once, and reading a
			// folder of a few names that often costs nothing.
			std::thread::sleep(Duration::from_millis(250));
		}
		Ok(ticket)
	}
}

impl Drop for Ticket {
	fn drop(&mut self) {
		let _ = fs::remove_file(&self.0);
	}
}

/// The queue's tickets in arrival order, those of dead processes dropped.
fn live_tickets(dir: &Path) -> Vec<PathBuf> {
	let Ok(entries) = fs::read_dir(dir) else {
		return Vec::new();
	};
	let mut live: Vec<PathBuf> = entries
		.flatten()
		.map(|e| e.path())
		.filter(|path| {
			let name = path
				.file_name()
				.and_then(|n| n.to_str())
				.unwrap_or_default();
			let Some(pid) = name
				.split_once('-')
				.and_then(|(_, pid)| pid.parse::<u32>().ok())
			else {
				return false;
			};
			let alive = Path::new(&format!("/proc/{pid}")).exists();
			if !alive {
				let _ = fs::remove_file(path);
			}
			alive
		})
		.collect();
	live.sort();
	live
}
