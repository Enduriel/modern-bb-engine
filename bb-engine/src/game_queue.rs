//! A place in the queue for the machine's headless game display, shared with the
//! probe runner that launches the original game (`/tmp/bb-probe-queue`). Probes
//! run the original in a headless gamescope, and two gamescopes at once can hang one's teardown, so tests that
//! run gamescope take a [`Ticket`] and wait their turn behind the probes, as the probes
//! do behind each other.
//!
//! The runner's semantics: a ticket is a file named `<nanoseconds, 20 digits>-<pid>`;
//! the first live one (its process still running) holds the display until its file
//! goes; tickets of dead processes are dropped.
//!
//! A process can hand its turn to the processes it starts: with `BB_GAME_QUEUE_TURN`
//! naming its ticket, theirs is that one once it is first in line (a test runner that
//! runs several gamescope test binaries one after another waits once, not once each).
//! The variable counts only when it names a live ticket of one of the process's
//! ancestors; otherwise it is ignored (with a line on stderr) and the process queues on
//! its own.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// The runner's queue.
const QUEUE: &str = "/tmp/bb-probe-queue";

/// A place in the queue; dropping it gives the display up, and so does the process's
/// end. A turn handed down (`BB_GAME_QUEUE_TURN`) stays the parent's to give up.
pub struct Ticket(Option<PathBuf>);

impl Ticket {
	/// Queue up and wait until first in line, for up to `max_wait`.
	pub fn wait(max_wait: Duration) -> Result<Ticket, String> {
		let dir = Path::new(QUEUE);
		let deadline = Instant::now() + max_wait;
		let busy = || format!("the game display stayed busy for {max_wait:?} ({QUEUE})");
		if let Some(held) = std::env::var_os("BB_GAME_QUEUE_TURN").map(PathBuf::from) {
			match handed_turn(dir, &held) {
				// The parent's place in line: wait there, not behind it (it keeps its
				// ticket until this process is done).
				Ok(()) => loop {
					let live = live_tickets(dir);
					if live.first() == Some(&held) {
						return Ok(Ticket(None));
					}
					if !live.contains(&held) {
						eprintln!("game queue: the turn handed down ({}) went before it came: queueing on our own", held.display());
						break;
					}
					if Instant::now() >= deadline {
						return Err(busy());
					}
					std::thread::sleep(Duration::from_millis(250));
				},
				Err(why) => eprintln!("game queue: BB_GAME_QUEUE_TURN={} ignored ({why}): queueing on our own", held.display()),
			}
		}
		fs::create_dir_all(dir).map_err(|e| format!("{QUEUE}: {e}"))?;
		let ns = SystemTime::now()
			.duration_since(UNIX_EPOCH)
			.map_err(|e| e.to_string())?
			.as_nanos();
		let path = dir.join(format!("{ns:020}-{}", std::process::id()));
		fs::File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?;
		let ticket = Ticket(Some(path.clone()));
		while live_tickets(dir).first() != Some(&path) {
			if Instant::now() >= deadline {
				return Err(busy());
			}
			// A quarter of a second: a turn handed on is taken at once, and reading a
			// folder of a few names that often costs nothing.
			std::thread::sleep(Duration::from_millis(250));
		}
		Ok(ticket)
	}
}

/// Whether `held` can be this process's turn: a live ticket in the queue whose process
/// is one of this one's ancestors (a stale or stray variable names some other's, or one
/// long gone).
fn handed_turn(dir: &Path, held: &Path) -> Result<(), String> {
	if held.parent() != Some(dir) {
		return Err(format!("not a ticket in {}", dir.display()));
	}
	let pid = ticket_pid(held).ok_or("not a ticket's name")?;
	if !live_tickets(dir).iter().any(|t| t == held) {
		return Err("no such live ticket".into());
	}
	let mut at = parent_of(std::process::id());
	while let Some(ancestor) = at.filter(|&p| p > 1) {
		if ancestor == pid {
			return Ok(());
		}
		at = parent_of(ancestor);
	}
	Err(format!("its process, {pid}, isn't an ancestor of this one"))
}

/// The pid in a ticket's name (`<nanoseconds>-<pid>`).
fn ticket_pid(ticket: &Path) -> Option<u32> {
	let name = ticket.file_name()?.to_str()?;
	name.split_once('-')?.1.parse().ok()
}

/// `pid`'s parent, from `/proc/<pid>/stat` (`pid (comm) state ppid ...`; comm may hold
/// spaces and parentheses).
fn parent_of(pid: u32) -> Option<u32> {
	let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
	stat.rsplit_once(')')?.1.split_whitespace().nth(1)?.parse().ok()
}

impl Drop for Ticket {
	fn drop(&mut self) {
		if let Some(path) = &self.0 {
			let _ = fs::remove_file(path);
		}
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
			let Some(pid) = ticket_pid(path) else {
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

#[cfg(test)]
mod tests {
	use super::*;

	/// A queue folder of the test's own, with a ticket for `pid`.
	fn queue_with(name: &str, pid: u32) -> (PathBuf, PathBuf) {
		let dir = std::env::temp_dir().join(format!("bb-game-queue-test-{}-{name}", std::process::id()));
		let _ = fs::remove_dir_all(&dir);
		fs::create_dir_all(&dir).unwrap();
		let ticket = dir.join(format!("{:020}-{pid}", 1));
		fs::File::create(&ticket).unwrap();
		(dir, ticket)
	}

	#[test]
	fn a_turn_handed_down_counts_only_from_a_live_ancestor() {
		let parent = parent_of(std::process::id()).expect("the test's parent");
		let (dir, ticket) = queue_with("parent", parent);
		assert_eq!(handed_turn(&dir, &ticket), Ok(()));
		// Elsewhere than the queue: not a ticket.
		assert!(handed_turn(Path::new("/nonexistent"), &ticket).is_err());
		let _ = fs::remove_dir_all(&dir);

		// The process itself isn't its own ancestor; pid 1 is everyone's, so never counts.
		for (name, pid) in [("self", std::process::id()), ("init", 1)] {
			let (dir, ticket) = queue_with(name, pid);
			assert!(handed_turn(&dir, &ticket).unwrap_err().contains("isn't an ancestor"), "{name}");
			let _ = fs::remove_dir_all(&dir);
		}

		// A ticket whose process is gone: dropped, so no turn. (A pid past the system's
		// highest is never alive; no process is started for one, which would hold this
		// binary's open files, the log files tests' locks among them, until it execs.)
		let max: u32 = fs::read_to_string("/proc/sys/kernel/pid_max").unwrap().trim().parse().unwrap();
		let (dir, ticket) = queue_with("gone", max + 1);
		assert_eq!(handed_turn(&dir, &ticket), Err("no such live ticket".to_string()));
		assert!(!ticket.exists());
		let _ = fs::remove_dir_all(&dir);
	}
}
