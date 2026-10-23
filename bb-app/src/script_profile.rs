//! `BB_SCRIPT_PROFILE=REPORT.md`: where the scripts' time goes while `bb` plays, by
//! function, file and mod ([`squirrel::ScriptProfiler`] over each frame's step: the
//! engine's frame and the page's calls into the scripts). Every
//! `BB_SCRIPT_PROFILE_SECS` seconds (default 10) the report of that window is added to
//! the file (emptied at the start), and a line of it goes to stderr;
//! `BB_SCRIPT_PROFILE_FRAMES=N` makes the windows N frames long instead (for scripted runs).
//! `BB_SCRIPT_PROFILE_SLOW_MS=MS` reports each frame whose step took at least `MS`
//! milliseconds on its own (a load, a battle's end), and no windows. Off (and costing
//! nothing) without the variable. The player's input, outside the step, isn't profiled.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use bb_engine::script_profile::{self, Owners};
use squirrel::{SQVM, ScriptProfiler};

pub struct ScriptProfiling {
	profiler: ScriptProfiler,
	owners: Owners,
	out: PathBuf,
	every: Duration,
	every_frames: Option<u64>,
	/// `BB_SCRIPT_PROFILE_SLOW_MS`: report only frames whose step took this long, alone.
	slow: Option<Duration>,
	/// Frames stepped, for the slow frames' titles.
	frames: u64,
	window_start: Instant,
	window: u32,
}

impl ScriptProfiling {
	/// Profiling as the environment asks, of the scripts in `archives`.
	pub fn from_env(archives: &[PathBuf]) -> Option<Self> {
		let out = std::env::var_os("BB_SCRIPT_PROFILE").filter(|p| !p.is_empty())?;
		if let Err(e) = std::fs::write(&out, "") {
			eprintln!("BB_SCRIPT_PROFILE: {e}");
		}
		let secs = std::env::var("BB_SCRIPT_PROFILE_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(10.0);
		Some(ScriptProfiling {
			profiler: ScriptProfiler::new(),
			owners: Owners::of(archives),
			out: PathBuf::from(out),
			every: Duration::from_secs_f64(secs),
			every_frames: std::env::var("BB_SCRIPT_PROFILE_FRAMES").ok().and_then(|s| s.parse().ok()),
			slow: std::env::var("BB_SCRIPT_PROFILE_SLOW_MS")
				.ok()
				.and_then(|s| s.parse::<f64>().ok())
				.filter(|ms| *ms >= 0.0)
				.map(|ms| Duration::from_secs_f64(ms / 1000.0)),
			frames: 0,
			window_start: Instant::now(),
			window: 0,
		})
	}

	/// Run a frame's step `f` on `vm`, profiled; at the window's end, report it.
	pub fn step<R>(&mut self, vm: &SQVM, f: impl FnOnce() -> R) -> R {
		self.frames += 1;
		if let Some(slow) = self.slow {
			return self.step_slow(vm, slow, f);
		}
		let result = vm.profiled(&mut self.profiler, f);
		let elapsed = self.window_start.elapsed();
		let done = match self.every_frames {
			Some(frames) => self.profiler.profile_runs() >= frames,
			None => elapsed >= self.every,
		};
		if done {
			self.window += 1;
			let profile = self.profiler.profile();
			let notes = format!("Window {} of bb's run, {:.1} s.\n", self.window, elapsed.as_secs_f64());
			self.write(&script_profile::report(&format!("bb, window {}", self.window), &notes, &profile, &self.owners, 30));
			let frames = profile.runs.max(1) as f64;
			eprintln!(
				"script profile: window {}, {} frames, {:.3} ms of the step a frame ({:.0} calls), report in {}",
				self.window,
				profile.runs,
				profile.wall_ns as f64 / 1e6 / frames,
				profile.events as f64 / 2.0 / frames,
				self.out.display()
			);
			self.profiler.clear();
			self.window_start = Instant::now();
		}
		result
	}

	/// A frame's step profiled alone, reported when it took at least `slow`.
	fn step_slow<R>(&mut self, vm: &SQVM, slow: Duration, f: impl FnOnce() -> R) -> R {
		self.profiler.clear();
		let start = Instant::now();
		let result = vm.profiled(&mut self.profiler, f);
		let took = start.elapsed();
		if took >= slow {
			self.window += 1;
			let profile = self.profiler.profile();
			let notes = format!(
				"Frame {} of bb's run, its step {:.1} ms; slow frame {}.\n",
				self.frames,
				took.as_secs_f64() * 1000.0,
				self.window
			);
			self.write(&script_profile::report(&format!("bb, frame {}", self.frames), &notes, &profile, &self.owners, 40));
			eprintln!(
				"script profile: frame {}, step {:.1} ms ({:.0} calls), report in {}",
				self.frames,
				took.as_secs_f64() * 1000.0,
				profile.events as f64 / 2.0,
				self.out.display()
			);
		}
		result
	}

	/// Add `text` to the report.
	fn write(&self, text: &str) {
		let written = std::fs::OpenOptions::new()
			.append(true)
			.open(&self.out)
			.and_then(|mut f| std::io::Write::write_all(&mut f, text.as_bytes()));
		if let Err(e) = written {
			eprintln!("BB_SCRIPT_PROFILE: {e}");
		}
	}
}
