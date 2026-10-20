//! The gameplay soak: a long campaign played headless and unattended, recording whatever
//! goes wrong.
//!
//! ```text
//! soak [--mods] [--scenario scenario.deserters] [--seed s] [--minutes 30]
//!      [--world-speed 3] [--save-every-days 3] [--watchdog-secs 180]
//!      [--data <install>/data] [--inject hang|panic|abort|stall|gameover] --out <dir>
//! ```
//!
//! The process started is the supervisor: it runs one session at a time as a child
//! process (this binary with `--child`) until the minutes are up, starting the next with
//! the next seed when one ends early (a crash, a hang, a stall it couldn't get out of).
//! A session boots the game (with the install's mods if `--mods`), starts a campaign
//! through the main menu's New Campaign page with the origin `--scenario`, and plays it
//! (`session.rs`): the world runs at `--world-speed`; events and contract screens get a
//! random (seeded) answer; the company travels to settlements, takes contracts and
//! pursues them, trades and hires through the town screen's handlers, hunts hostile
//! parties in sight; battles are fought by the AI on both sides; every
//! `--save-every-days` the campaign is saved (`bb_probe_soak_*`, in
//! `target/soak-saves/`) and loaded back, and what must survive is compared. On game
//! over it starts a new campaign with the next seed.
//!
//! Output in `--out`:
//! - `events.jsonl`: one JSON object per line (`kind` says what): script errors (message,
//!   call stack, the innermost frames' locals), compile errors, panics (with backtrace),
//!   engine errors from a frame, errors the scripts log, stalls (with the screens up),
//!   save/load failures and invariant mismatches, stubs first called, memory every
//!   minute, battles, contracts, campaigns, keep-alive top-ups, session ends, crashes and
//!   hangs (with the native stacks), and a `summary` at the end of each session and of
//!   the run.
//! - `summary.json`: the run's totals and the errors by signature.
//! - `actions-<n>.log`: session n's play, line by line (what it did and why).
//! - `game-<n>.log`: the game's own log lines (logInfo/logWarning/logError), capped.
//! - `stderr-<n>.log`: the session's stderr (script error stacks as the terminal shows
//!   them, panics).
//! - `hang-<n>.txt`: the native stacks of a session the watchdog found stuck.

mod battle;
mod handlers;
mod record;
mod saveload;
mod session;
mod supervisor;
mod world;

use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Clone, Debug)]
pub struct Options {
	pub data: PathBuf,
	pub mods: bool,
	pub scenario: String,
	pub seed: String,
	pub minutes: f64,
	pub world_speed: f64,
	pub save_every_days: i64,
	pub watchdog_secs: u64,
	pub out: PathBuf,
	/// The session number, for a child.
	pub child: Option<u32>,
	/// When the run ends (Unix seconds), for a child.
	pub deadline: Option<u64>,
	/// A fault to inject into the first session 20 s in, to test the runner itself:
	/// `hang` (a script that never returns), `panic`, `abort`, `stall` (the frames stop running) or `gameover` (the
	/// company dismissed).
	pub inject: Option<String>,
}

fn options() -> Result<Options, String> {
	let mut o = Options {
		data: PathBuf::from(bb_engine::harness::STEAM_DATA_DIR),
		mods: false,
		scenario: "scenario.early_access".into(),
		seed: "soak".into(),
		minutes: 30.0,
		world_speed: 3.0,
		save_every_days: 3,
		watchdog_secs: 180,
		out: PathBuf::new(),
		child: None,
		deadline: None,
		inject: None,
	};
	let mut args = std::env::args().skip(1);
	while let Some(a) = args.next() {
		let mut value = || args.next().ok_or(format!("{a} needs a value"));
		let num = |v: String| v.parse::<f64>().map_err(|e| format!("{a}: {e}"));
		match a.as_str() {
			"--data" => o.data = PathBuf::from(value()?),
			"--mods" => o.mods = true,
			"--scenario" => o.scenario = value()?,
			"--seed" => o.seed = value()?,
			"--minutes" => o.minutes = num(value()?)?,
			"--world-speed" => o.world_speed = num(value()?)?,
			"--save-every-days" => o.save_every_days = num(value()?)? as i64,
			"--watchdog-secs" => o.watchdog_secs = num(value()?)? as u64,
			"--out" => o.out = PathBuf::from(value()?),
			"--child" => o.child = Some(num(value()?)? as u32),
			"--deadline" => o.deadline = Some(num(value()?)? as u64),
			"--inject" => o.inject = Some(value()?),
			"-h" | "--help" => return Err("usage: soak [--mods] [--scenario id] [--seed s] [--minutes m] [--world-speed x] [--save-every-days d] [--watchdog-secs s] [--data dir] [--inject hang|panic|abort|stall|gameover] --out dir".into()),
			other => return Err(format!("unknown argument {other}")),
		}
	}
	if o.out.as_os_str().is_empty() {
		return Err("--out <dir> is required".into());
	}
	Ok(o)
}

fn main() -> ExitCode {
	let o = match options() {
		Ok(o) => o,
		Err(e) => {
			eprintln!("{e}");
			return ExitCode::from(2);
		}
	};
	match o.child {
		Some(n) => session::main(&o, n),
		None => supervisor::main(&o),
	}
}
