//! The supervisor: runs sessions (this binary with `--child`) one after another until
//! the run's minutes are up, watches each (a heartbeat line at least every
//! `--watchdog-secs`, or its native stacks are dumped and it is killed), samples its
//! memory every minute, and adds up what the sessions report.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, ExitCode, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::Options;
use crate::record::{self, Json, quote};

/// A session's exit code when the run's time is up.
pub const EXIT_DONE: i32 = 0;
/// A session's exit code when it gave up on a stall it couldn't get out of.
pub const EXIT_STALLED: i32 = 3;
/// A session's exit code after a Rust panic (recorded by the panic hook).
pub const EXIT_PANIC: i32 = 4;
/// A session's exit code when the game didn't boot or its campaign didn't start.
pub const EXIT_BOOT: i32 = 5;

fn unix_now() -> u64 {
	SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[derive(Default)]
struct Totals {
	/// The counters of each session's last `STAT` line, added up.
	stats: BTreeMap<String, i64>,
	/// (kind, signature) → occurrences.
	signatures: BTreeMap<(String, String), u64>,
	sessions: u32,
	crashes: u32,
	hangs: u32,
	stalls_given_up: u32,
	panics: u32,
	boot_failures: u32,
	peak_rss_mb: f64,
}

pub fn main(o: &Options) -> ExitCode {
	if let Err(e) = std::fs::create_dir_all(&o.out) {
		eprintln!("--out {}: {e}", o.out.display());
		return ExitCode::from(2);
	}
	let events = o.out.join("events.jsonl");
	if let Err(e) = record::open(&events, 0, &o.seed) {
		eprintln!("{}: {e}", events.display());
		return ExitCode::from(2);
	}
	let started = Instant::now();
	let deadline = unix_now() + (o.minutes * 60.0).round().max(1.0) as u64;
	Json::new("run_start")
		.int("pid", std::process::id() as i64)
		.str("scenario", &o.scenario)
		.int("mods", o.mods as i64)
		.num("minutes", o.minutes)
		.num("world_speed", o.world_speed)
		.int("save_every_days", o.save_every_days)
		.str("data", &o.data.to_string_lossy())
		.emit();
	let mut totals = Totals::default();
	let mut boot_failures_in_a_row = 0;
	let mut n = 0;
	while unix_now() + 5 < deadline {
		n += 1;
		let seed = if n == 1 { o.seed.clone() } else { format!("{}-{n}", o.seed) };
		eprintln!("soak: session {n}, seed {seed}");
		let code = run_child(o, n, &seed, deadline, &mut totals);
		boot_failures_in_a_row = if code == Some(EXIT_BOOT) { boot_failures_in_a_row + 1 } else { 0 };
		if boot_failures_in_a_row >= 3 {
			Json::new("run_abort").str("why", "three sessions in a row failed to boot").emit();
			break;
		}
	}
	let summary = summary(o, &totals, started.elapsed());
	let _ = std::fs::write(o.out.join("summary.json"), format!("{summary}\n"));
	Json::new("summary").raw("run", &summary).emit();
	eprintln!("soak: done, {} session(s); {}", totals.sessions, o.out.join("summary.json").display());
	ExitCode::SUCCESS
}

/// Run session `n`; its exit code (None if killed or signalled).
fn run_child(o: &Options, n: u32, seed: &str, deadline: u64, totals: &mut Totals) -> Option<i32> {
	totals.sessions += 1;
	let stderr_path = o.out.join(format!("stderr-{n}.log"));
	let stderr = match File::create(&stderr_path) {
		Ok(f) => f,
		Err(e) => {
			Json::new("crash").int("child", n as i64).str("why", &format!("stderr log: {e}")).emit();
			return None;
		}
	};
	let mut other_out = OpenOptions::new().append(true).open(&stderr_path).ok();
	let exe = std::env::current_exe().unwrap_or_else(|_| "soak".into());
	let mut cmd = Command::new(exe);
	cmd.arg("--child")
		.arg(n.to_string())
		.arg("--deadline")
		.arg(deadline.to_string())
		.args(["--data", &o.data.to_string_lossy()])
		.args(["--scenario", &o.scenario])
		.args(["--seed", seed])
		.args(["--world-speed", &o.world_speed.to_string()])
		.args(["--save-every-days", &o.save_every_days.to_string()])
		.args(["--out", &o.out.to_string_lossy()])
		.stdin(Stdio::null())
		.stdout(Stdio::piped())
		.stderr(Stdio::from(stderr));
	if o.mods {
		cmd.arg("--mods");
	}
	if let Some(inject) = o.inject.as_ref().filter(|_| n == 1) {
		cmd.args(["--inject", inject]);
	}
	let mut child = match cmd.spawn() {
		Ok(c) => c,
		Err(e) => {
			Json::new("crash").int("child", n as i64).str("why", &format!("spawn: {e}")).emit();
			return None;
		}
	};
	let pid = child.id();
	Json::new("session_start").int("child", n as i64).int("pid", pid as i64).str("child_seed", seed).emit();
	let (tx, rx) = mpsc::channel::<String>();
	let stdout = child.stdout.take();
	let reader = std::thread::spawn(move || {
		if let Some(out) = stdout {
			for line in BufReader::new(out).lines().map_while(Result::ok) {
				if tx.send(line).is_err() {
					break;
				}
			}
		}
	});
	let watchdog = Duration::from_secs(o.watchdog_secs.max(10));
	let mut last_beat = Instant::now();
	let mut last_state = String::from("(no heartbeat yet)");
	let mut last_memory = Instant::now() - Duration::from_secs(55);
	let mut stats: BTreeMap<String, i64> = BTreeMap::new();
	let mut killed: Option<&str> = None;
	let status = loop {
		match rx.recv_timeout(Duration::from_millis(500)) {
			Ok(line) => {
				if let Some(rest) = line.strip_prefix("SOAK ") {
					let (tag, rest) = rest.split_once(' ').unwrap_or((rest, ""));
					match tag {
						"HB" => {
							last_beat = Instant::now();
							last_state = rest.to_string();
						}
						"STAT" => {
							last_beat = Instant::now();
							stats = rest
								.split_whitespace()
								.filter_map(|kv| kv.split_once('='))
								.filter_map(|(k, v)| Some((k.to_string(), v.parse().ok()?)))
								.collect();
						}
						"SIG" => {
							let (kind, sig) = rest.split_once('\t').unwrap_or((rest, ""));
							*totals.signatures.entry((kind.to_string(), sig.to_string())).or_default() += 1;
						}
						_ => {}
					}
				} else if let Some(f) = other_out.as_mut() {
					let _ = writeln!(f, "{line}");
				}
			}
			Err(mpsc::RecvTimeoutError::Timeout) | Err(mpsc::RecvTimeoutError::Disconnected) => {}
		}
		if last_memory.elapsed() >= Duration::from_secs(60) {
			last_memory = Instant::now();
			if let Ok(status) = std::fs::read_to_string(format!("/proc/{pid}/status")) {
				let (rss, peak) = record::rss_mb(&status);
				totals.peak_rss_mb = totals.peak_rss_mb.max(peak);
				Json::new("memory")
					.int("child", n as i64)
					.num("rss_mb", rss)
					.num("peak_rss_mb", peak)
					.str("state", &last_state)
					.emit();
			}
		}
		if let Ok(Some(status)) = child.try_wait() {
			break Some(status);
		}
		let overrun = unix_now() > deadline + o.watchdog_secs.max(10);
		if last_beat.elapsed() > watchdog || overrun {
			let why = if overrun { "the run's time is up and the session didn't end" } else { "no heartbeat" };
			let stacks = dump_stacks(o, n, pid);
			Json::new("hang")
				.int("child", n as i64)
				.str("why", why)
				.num("silent_secs", last_beat.elapsed().as_secs_f64())
				.str("last_state", &last_state)
				.str("stacks_file", &stacks)
				.emit();
			totals.hangs += 1;
			killed = Some(why);
			let _ = child.kill();
			break child.wait().ok();
		}
	};
	let _ = reader.join();
	// Lines the reader passed on after the last poll.
	while let Ok(line) = rx.try_recv() {
		if let Some(rest) = line.strip_prefix("SOAK STAT ") {
			stats = rest
				.split_whitespace()
				.filter_map(|kv| kv.split_once('='))
				.filter_map(|(k, v)| Some((k.to_string(), v.parse().ok()?)))
				.collect();
		} else if let Some(rest) = line.strip_prefix("SOAK SIG ") {
			let (kind, sig) = rest.split_once('\t').unwrap_or((rest, ""));
			*totals.signatures.entry((kind.to_string(), sig.to_string())).or_default() += 1;
		}
	}
	for (k, v) in &stats {
		*totals.stats.entry(k.clone()).or_default() += v;
	}
	let code = status.and_then(|s| s.code());
	let signal = status.and_then(|s| s.signal());
	match (killed, code) {
		(Some(_), _) => {}
		(None, Some(EXIT_DONE)) => {}
		(None, Some(EXIT_STALLED)) => totals.stalls_given_up += 1,
		(None, Some(EXIT_PANIC)) => totals.panics += 1,
		(None, Some(EXIT_BOOT)) => totals.boot_failures += 1,
		_ => {
			totals.crashes += 1;
			let tail = tail(&stderr_path, 40);
			*totals
				.signatures
				.entry(("crash".into(), format!("exit {code:?} signal {signal:?}")))
				.or_default() += 1;
			Json::new("crash")
				.int("child", n as i64)
				.str("exit", &format!("{code:?}"))
				.str("signal", &format!("{signal:?}"))
				.str("last_state", &last_state)
				.str("stderr_tail", &tail)
				.emit();
		}
	}
	Json::new("session_end")
		.int("child", n as i64)
		.str("exit", &format!("{code:?}"))
		.str("signal", &format!("{signal:?}"))
		.str("killed", killed.unwrap_or(""))
		.str("last_state", &last_state)
		.raw("stats", &stats_json(&stats))
		.emit();
	code
}

/// The native stacks of a stuck session (`eu-stack`, the session allowed it to attach);
/// the file's path, or why there is none.
fn dump_stacks(o: &Options, n: u32, pid: u32) -> String {
	let path = o.out.join(format!("hang-{n}.txt"));
	let out = Command::new("timeout").args(["60", "eu-stack", "-p", &pid.to_string()]).output();
	match out {
		Ok(out) => {
			let mut text = out.stdout;
			text.extend_from_slice(&out.stderr);
			match std::fs::write(&path, text) {
				Ok(()) => path.to_string_lossy().into_owned(),
				Err(e) => format!("(writing {}: {e})", path.display()),
			}
		}
		Err(e) => format!("(eu-stack: {e})"),
	}
}

/// The last `lines` lines of the file at `path`.
fn tail(path: &std::path::Path, lines: usize) -> String {
	let text = std::fs::read(path).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
	let all: Vec<&str> = text.lines().collect();
	all[all.len().saturating_sub(lines)..].join("\n")
}

fn stats_json(stats: &BTreeMap<String, i64>) -> String {
	let fields: Vec<String> = stats.iter().map(|(k, v)| format!("{}:{v}", quote(k))).collect();
	format!("{{{}}}", fields.join(","))
}

fn summary(o: &Options, t: &Totals, elapsed: Duration) -> String {
	let mut sigs: Vec<(&(String, String), &u64)> = t.signatures.iter().collect();
	sigs.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
	let errors: Vec<String> = sigs
		.iter()
		.map(|((kind, sig), count)| format!("{{\"kind\":{},\"signature\":{},\"count\":{count}}}", quote(kind), quote(sig)))
		.collect();
	let mut by_kind: BTreeMap<&str, u64> = BTreeMap::new();
	for ((kind, _), count) in &t.signatures {
		*by_kind.entry(kind).or_default() += count;
	}
	let kinds: Vec<String> = by_kind.iter().map(|(k, v)| format!("{}:{v}", quote(k))).collect();
	format!(
		"{{\"seed\":{},\"scenario\":{},\"mods\":{},\"minutes\":{:.1},\"sessions\":{},\"crashes\":{},\"hangs\":{},\"stalls_given_up\":{},\"panics\":{},\"boot_failures\":{},\"peak_rss_mb\":{:.0},\"totals\":{},\"errors_by_kind\":{{{}}},\"errors_by_signature\":[{}]}}",
		quote(&o.seed),
		quote(&o.scenario),
		o.mods,
		elapsed.as_secs_f64() / 60.0,
		t.sessions,
		t.crashes,
		t.hangs,
		t.stalls_given_up,
		t.panics,
		t.boot_failures,
		t.peak_rss_mb,
		stats_json(&t.stats),
		kinds.join(","),
		errors.join(",")
	)
}
