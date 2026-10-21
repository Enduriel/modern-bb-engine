//! Whether the page gets a turn this frame, or rests: stays parked across the game's
//! frames, with no park round trip and no frame begun, until it has work again.
//!
//! The game in a window parks the page for each frame's scripts and lets it run between
//! frames ([`UiSession::step_unparked`](super::UiSession::step_unparked)), and begins a
//! page frame after each draw ([`PageFrames::Game`](super::PageFrames)). On a still
//! screen (the menu, a town, the paused map) that round trip and the begin frame were
//! most of what bb did: ~13 thread wake-ups a frame across the game's and the page's
//! processes, for a page with nothing to do. Chromium can't say when it needs frames
//! (CEF doesn't pass on viz's "needs begin frame"), so the page says what it has to do
//! when it parks (`page_work.js`): whether a task of its own ran since it last parked
//! (a timer, an animation frame, an event: its changes may still need a frame), whether
//! it waits for an animation frame, runs a CSS animation or transition, edits text (the
//! caret blinks) or loads fonts, and when its next timer is due. The game adds what
//! only it sees: its calls into the page, the input it sent, `coui://` loads finished,
//! a resize, paints that landed. A frame's keys are the exception: their hand-off
//! unparks the page until it has seen them, which is their turn; after it, a key the
//! browser does nothing visible with by default (W held to scroll the map, a modifier)
//! gives the page no more turns unless its listeners did something (`page_work.js`). The page gets a turn while any of those says so, and
//! for [`SETTLE`] after the last; then it rests until one does again (a timer coming
//! due, input, a call from the scripts).
//!
//! Resting changes nothing the page or the scripts can see: the scripts' calls reach a
//! resting page at once (it's parked: they travel as its park's answers), a timer
//! fires in the turn its due time falls in, as it did, and a frame's keys still reach
//! the page before the states (that hand-off unparks it on its own).

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use crate::json::Json;

/// How long the page keeps its turns after it last had work. A change the page made
/// reaches the screen through Chromium's pipeline (main frame, commit, raster,
/// activation, draw, capture: ~3 ms measured, longer when pictures decode), which says
/// nothing until the paint lands. Each paint extends the turns, so this only covers a
/// paint still in that pipeline when the page has gone quiet. Measured over whole
/// sessions (menu, map, towns, battles, modded and vanilla; `BB_PAGE_REST_LOG`), such a
/// paint landed at most 11.3 ms after the page went quiet at 180 Hz and 17.5 ms at 60 Hz
/// (a frame or two): this is about three times that.
const SETTLE: Duration = Duration::from_millis(50);

/// The longest a frame's interval counts as (a timer due within the next frame gets
/// this frame's turn): a stalled loop shouldn't make every timer look due.
const MAX_INTERVAL: Duration = Duration::from_millis(100);

/// What the page said when it last parked.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Report {
	/// It has work now (`page_work.js` says what), or said nothing we could read.
	pub busy: bool,
	/// When its next timer is due.
	pub timer: Option<Instant>,
	/// How many keydown and keyup events it has seen, if it said.
	pub keys: Option<i64>,
}

impl Report {
	/// Read the park request's body (`{"busy": reason | null, "timer": ms | null}`),
	/// received at `now`. Anything else is busy: a page we can't read never rests.
	pub fn parse(body: &str, now: Instant) -> Report {
		let busy = Report { busy: true, timer: None, keys: None };
		let Ok(json) = Json::parse(body) else { return busy };
		let Some(reason) = json.get("busy") else { return busy };
		let timer = match json.get("timer") {
			Some(Json::Int(ms)) => Some(now + Duration::from_millis((*ms).max(0) as u64)),
			Some(Json::Float(ms)) if ms.is_finite() => Some(now + Duration::from_secs_f64(ms.max(0.0) / 1000.0)),
			_ => None,
		};
		let keys = json.get("keys").and_then(Json::as_i64);
		Report { busy: !matches!(reason, Json::Null), timer, keys }
	}
}

/// The decision's state (see the module notes), on the game thread.
pub(super) struct PageRest {
	/// Whether the page may rest at all (`BB_PAGE_REST`; not while DevTools serve it).
	enabled: bool,
	/// Something the game did gives the page work: a call into it, input, a resize.
	woken: Cell<bool>,
	report: Cell<Report>,
	/// The paints and finished `coui://` loads counted at the last decision.
	paints: Cell<u64>,
	loads: Cell<u64>,
	/// When the page last had work.
	active: Cell<Option<Instant>>,
	/// Since when nothing but paints gave it work (for the log: a paint landing then is
	/// one that resting at once would have kept off the screen).
	quiet: Cell<Option<Instant>>,
	/// When the last decision was made, and the interval between the last two.
	decided: Cell<Option<Instant>>,
	interval: Cell<Duration>,
	/// Whether the page rests now: parked by the last decision, with no turn since.
	resting: Cell<bool>,
	/// How many decisions so far left it resting.
	rests: Cell<u64>,
	/// Log each decision that changes whether the page rests, and every 1,000 decisions
	/// what gave the page its turns (`BB_PAGE_REST_LOG`, level 1); at level 2
	/// (`BB_PAGE_REST_LOG=2`) every decision.
	log: u8,
	/// For the log: why the page last said it's busy, and the causes counted.
	reason: RefCell<String>,
	tally: RefCell<BTreeMap<String, u32>>,
}

impl PageRest {
	pub fn new(enabled: bool) -> PageRest {
		PageRest {
			enabled,
			woken: Cell::new(true),
			report: Cell::new(Report { busy: true, timer: None, keys: None }),
			paints: Cell::new(0),
			loads: Cell::new(0),
			active: Cell::new(None),
			quiet: Cell::new(None),
			decided: Cell::new(None),
			interval: Cell::new(Duration::ZERO),
			resting: Cell::new(false),
			rests: Cell::new(0),
			log: match std::env::var("BB_PAGE_REST_LOG").as_deref() {
				Err(_) | Ok("0") => 0,
				Ok("2") => 2,
				Ok(_) => 1,
			},
			reason: RefCell::new(String::new()),
			tally: RefCell::new(BTreeMap::new()),
		}
	}

	/// Whether the page may rest: with the game's frames, unless `BB_PAGE_REST=0`, and
	/// not while DevTools are served (they reach the page through its main thread, which
	/// a resting page holds) unless `BB_PAGE_REST=1`.
	pub fn wanted(external_frames: bool, devtools: bool) -> bool {
		match std::env::var("BB_PAGE_REST").as_deref() {
			Ok("0") => false,
			Ok("1") => external_frames,
			_ => external_frames && !devtools,
		}
	}

	/// The game gave the page work.
	pub fn wake(&self) {
		self.woken.set(true);
	}

	/// The page parked and said what it has to do.
	pub fn reported(&self, report: Report) {
		self.report.set(report);
	}

	/// The page parked and said what it has to do, as `body` received at `now` (see
	/// [`Report::parse`]).
	pub fn reported_text(&self, body: &str, now: Instant) {
		self.note_reason(body);
		self.reported(Report::parse(body, now));
	}

	/// The game's call into the waiting page ran, after which it said what it has to do
	/// (`body`, as a park's, received at `now`). Its timers are as it says now; work it
	/// has (the call changed what it shows, started an animation) holds until the next
	/// decision, whatever later calls say.
	pub fn called(&self, body: &str, now: Instant) {
		let report = Report::parse(body, now);
		if report.busy {
			self.note_reason(body);
			self.woken.set(true);
		}
		let last = self.report.get();
		self.report.set(Report { busy: last.busy, timer: report.timer, keys: report.keys.or(last.keys) });
	}

	fn note_reason(&self, body: &str) {
		if self.log > 0 {
			let reason = Json::parse(body).ok().and_then(|j| j.get("busy").and_then(|r| r.as_str().map(str::to_string)));
			*self.reason.borrow_mut() = reason.unwrap_or_else(|| "?".into());
		}
	}

	/// How many keydown and keyup events the page had seen when it last said, if it did.
	pub fn keys_seen(&self) -> Option<i64> {
		self.report.get().keys
	}

	pub fn resting(&self) -> bool {
		self.resting.get()
	}

	/// How many game frames so far the page rested through.
	pub fn rests(&self) -> u64 {
		self.rests.get()
	}

	/// The page got a turn outside [`decide`](Self::decide) (it was let go on).
	pub fn ran(&self) {
		self.resting.set(false);
	}

	/// At the end of a game frame's step, with the page parked: whether it gets a turn
	/// until the next frame parks it (`true`), or rests. `paints` and `loads` are the
	/// counts so far of the page's paints and of its finished `coui://` loads.
	pub fn decide(&self, now: Instant, paints: u64, loads: u64) -> bool {
		if let Some(last) = self.decided.replace(Some(now)) {
			self.interval.set(now.saturating_duration_since(last).min(MAX_INTERVAL));
		}
		let report = self.report.get();
		let painted = self.paints.replace(paints) != paints;
		let loaded = self.loads.replace(loads) != loads;
		let woken = self.woken.replace(false);
		let timer = report.timer.is_some_and(|due| due <= now + self.interval.get());
		let cause = woken || loaded || report.busy || timer;
		let work = !self.enabled || cause || painted;
		if work {
			self.active.set(Some(now));
		}
		if self.log > 0 && painted && !cause {
			if let Some(since) = self.quiet.get() {
				let ms = now.saturating_duration_since(since).as_secs_f64() * 1000.0;
				eprintln!("page rest: a paint landed {ms:.1} ms after the page went quiet");
			}
		}
		if cause {
			self.quiet.set(None);
		} else if self.quiet.get().is_none() {
			self.quiet.set(Some(now));
		}
		let turn = work || self.active.get().is_none_or(|at| now.saturating_duration_since(at) < SETTLE);
		if self.log > 0 {
			self.count(&[
				(woken, "woken"),
				(painted, "painted"),
				(loaded, "loaded"),
				(report.busy, &format!("busy:{}", self.reason.borrow())),
				(timer, "timer"),
				(turn && !work, "settling"),
				(!turn, "rested"),
			]);
		}
		if self.log > 1 || (self.log > 0 && turn == self.resting.get()) {
			let busy = format!("busy:{}", self.reason.borrow());
			let why = [(woken, "woken"), (painted, "painted"), (loaded, "loaded"), (report.busy, busy.as_str()), (timer, "timer")]
				.iter()
				.filter(|(on, _)| *on)
				.map(|(_, name)| *name)
				.collect::<Vec<_>>()
				.join(",");
			let next = report.timer.map(|due| due.saturating_duration_since(now).as_secs_f64() * 1000.0);
			let state = match (turn, work) {
				(false, _) => "rests",
				(true, true) => "turn",
				(true, false) => "settling",
			};
			eprintln!("page rest: {state} ({why}; next timer {next:?} ms)");
		}
		self.resting.set(!turn);
		if !turn {
			self.rests.set(self.rests.get() + 1);
		}
		turn
	}
}

impl PageRest {
	/// For the log: count the causes that hold, and every 1,000 decisions print them.
	fn count(&self, causes: &[(bool, &str)]) {
		let mut tally = self.tally.borrow_mut();
		for (_, name) in causes.iter().filter(|(on, _)| *on) {
			*tally.entry(name.to_string()).or_default() += 1;
		}
		let decisions = tally.entry("decisions".into()).or_default();
		*decisions += 1;
		if *decisions >= 1000 {
			let line = tally.iter().map(|(k, v)| format!("{k} {v}")).collect::<Vec<_>>().join(", ");
			eprintln!("page rest: of the last decisions: {line}");
			tally.clear();
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn quiet(timer: Option<Instant>) -> Report {
		Report { busy: false, timer, keys: None }
	}

	#[test]
	fn reports_read_the_page_s_answer() {
		let now = Instant::now();
		assert_eq!(Report::parse(r#"{"busy":null,"timer":null}"#, now), quiet(None));
		assert_eq!(Report::parse(r#"{"busy":null,"timer":250}"#, now), quiet(Some(now + Duration::from_millis(250))));
		assert_eq!(Report::parse(r#"{"busy":null,"timer":1.5}"#, now), quiet(Some(now + Duration::from_micros(1500))));
		assert_eq!(Report::parse(r#"{"busy":"frames","timer":null}"#, now), Report { busy: true, timer: None, keys: None });
		assert_eq!(Report::parse(r#"{"busy":null,"timer":null,"keys":7}"#, now).keys, Some(7));
		assert!(Report::parse("", now).busy, "a page that says nothing never rests");
		assert!(Report::parse(r#"{"timer":5}"#, now).busy);
	}

	#[test]
	fn a_quiet_page_rests_after_settling() {
		let rest = PageRest::new(true);
		let t0 = Instant::now();
		let frame = Duration::from_micros(5556);
		assert!(rest.decide(t0, 0, 0), "the first frame is a turn (it starts woken)");
		rest.reported(quiet(None));
		let mut t = t0;
		while t < t0 + SETTLE {
			t += frame;
			assert!(rest.decide(t, 0, 0) || t >= t0 + SETTLE, "settling");
		}
		t += frame;
		assert!(!rest.decide(t, 0, 0));
		assert!(rest.resting());
		// It stays resting with nothing to do.
		assert!(!rest.decide(t + frame * 1000, 0, 0));
	}

	#[test]
	fn work_gives_turns() {
		let rest = PageRest::new(true);
		let t0 = Instant::now();
		rest.decide(t0, 0, 0);
		rest.reported(quiet(None));
		let later = t0 + SETTLE * 3;
		assert!(!rest.decide(later, 0, 0));
		assert!(rest.decide(later + Duration::from_millis(1), 1, 0), "a paint landed");
		let later = later + SETTLE * 3;
		assert!(!rest.decide(later, 1, 0));
		assert!(rest.decide(later + Duration::from_millis(1), 1, 1), "a load finished");
		let later = later + SETTLE * 3;
		assert!(!rest.decide(later, 1, 1));
		rest.wake();
		assert!(rest.decide(later + Duration::from_millis(1), 1, 1), "the game gave it work");
		let later = later + SETTLE * 3;
		assert!(!rest.decide(later, 1, 1));
		rest.reported(Report { busy: true, timer: None, keys: None });
		assert!(rest.decide(later + Duration::from_millis(1), 1, 1), "it said it's busy");
	}

	#[test]
	fn a_timer_s_turn_is_the_frame_its_due_time_falls_in() {
		let rest = PageRest::new(true);
		let t0 = Instant::now();
		let frame = Duration::from_millis(10);
		rest.decide(t0, 0, 0);
		rest.reported(quiet(None));
		let t1 = t0 + SETTLE * 2;
		// A frame before, so the interval is a frame's.
		assert!(!rest.decide(t1 - frame, 0, 0));
		rest.reported(quiet(Some(t1 + frame * 5 + Duration::from_millis(3))));
		let mut t = t1;
		let mut turns = Vec::new();
		for i in 0..10 {
			if rest.decide(t, 0, 0) {
				turns.push(i);
				break;
			}
			t += frame;
		}
		// Due between the decisions at frames 5 and 6: frame 5's turn runs until frame 6 parks it.
		assert_eq!(turns, [5]);
	}

	#[test]
	fn a_call_that_changed_nothing_leaves_the_page_resting() {
		let rest = PageRest::new(true);
		let t0 = Instant::now();
		rest.decide(t0, 0, 0);
		rest.reported(quiet(None));
		let t1 = t0 + SETTLE * 3;
		assert!(!rest.decide(t1, 0, 0));
		// Calls that changed nothing (vanilla's tooltip hover with a still mouse).
		rest.called(r#"{"busy":null,"timer":null}"#, t1);
		assert!(!rest.decide(t1 + Duration::from_millis(5), 0, 0));
		// One that set a timer: its turn comes when it's due.
		rest.called(r#"{"busy":null,"timer":200}"#, t1);
		assert!(!rest.decide(t1 + Duration::from_millis(10), 0, 0));
		assert!(rest.decide(t1 + Duration::from_millis(198), 0, 0));
		let t2 = t1 + Duration::from_millis(198) + SETTLE * 3;
		rest.reported(quiet(None));
		assert!(!rest.decide(t2, 0, 0));
		// One that changed the page, then one that didn't: the change still needs a frame.
		rest.called(r#"{"busy":"changed","timer":null}"#, t2);
		rest.called(r#"{"busy":null,"timer":null}"#, t2);
		assert!(rest.decide(t2 + Duration::from_millis(5), 0, 0));
		// A page that can't say what it has to do gets its turn.
		let t3 = t2 + SETTLE * 3;
		rest.reported(quiet(None));
		assert!(!rest.decide(t3, 0, 0));
		rest.called("", t3);
		assert!(rest.decide(t3 + Duration::from_millis(5), 0, 0));
	}

	#[test]
	fn a_page_that_may_not_rest_always_gets_its_turn() {
		let rest = PageRest::new(false);
		let t0 = Instant::now();
		rest.decide(t0, 0, 0);
		rest.reported(quiet(None));
		assert!(rest.decide(t0 + SETTLE * 10, 0, 0));
		assert!(!rest.resting());
	}
}
