//! A window surface the system won't configure or present to, and its retries.
//!
//! wgpu sends a failed `Surface::configure` or present to its uncaught-error handler,
//! which panics: a game on Direct3D 12 has ended so ("Invalid surface").
//! Its cause is dealt with where it lies (no exclusive fullscreen on Windows, see
//! `platform::exclusive_fullscreen`). This is the backstop for what the system may still
//! refuse: a swapchain DXGI won't resize or make for the window during a display change,
//! a lost Vulkan surface. Such a refusal passes: the game keeps running, draws nothing
//! to the window, and configures it again every frame until it takes. A failed
//! configure leaves wgpu with no swapchain (Direct3D 12's `ResizeBuffers` failing drops
//! it), so each retry makes one anew: a new surface would add nothing. If the window
//! still can't be drawn to after [`GIVE_UP`], the game stops with the error, shown to
//! the player as every start failure is, rather than going on unseen.
//!
//! The first failure, the end of the trouble and the error at the give-up are logged
//! (log.html and stderr); the retries in between are not, one per frame.

use std::time::{Duration, Instant};

/// How long the window may refuse its surface before the game stops: well past a
/// display's mode change (a second or two on a monitor that resyncs).
pub const GIVE_UP: Duration = Duration::from_secs(10);

/// The surface's configure or present failed and hasn't been configured since.
#[derive(Debug)]
pub struct SurfaceTrouble {
	since: Instant,
	attempts: u32,
	last: String,
}

/// What to log after an attempt: nothing, a warning (the trouble starts), or that it's
/// over.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
	Fine,
	Warn(String),
	Recovered(String),
}

impl SurfaceTrouble {
	/// `what` (configure, present) failed with `error` at `now`; `trouble` is the
	/// window's state, `None` while it's fine.
	pub fn failed(trouble: &mut Option<SurfaceTrouble>, what: &str, error: &str, now: Instant) -> Outcome {
		match trouble {
			Some(t) => {
				t.attempts += 1;
				t.last = format!("{what}: {error}");
				Outcome::Fine
			}
			None => {
				*trouble = Some(SurfaceTrouble { since: now, attempts: 1, last: format!("{what}: {error}") });
				Outcome::Warn(format!(
					"the window's surface failed ({what}): {error}; configuring it again each frame"
				))
			}
		}
	}

	/// The surface was configured at `now`: the trouble, if any, is over.
	pub fn configured(trouble: &mut Option<SurfaceTrouble>, now: Instant) -> Outcome {
		match trouble.take() {
			None => Outcome::Fine,
			Some(t) => Outcome::Recovered(format!(
				"the window's surface is configured again, after {} failed attempts in {} ms",
				t.attempts,
				now.duration_since(t.since).as_millis()
			)),
		}
	}

	/// Whether the game should stop at `now`, and the error it stops with.
	pub fn given_up(&self, now: Instant) -> Option<String> {
		(now.duration_since(self.since) >= GIVE_UP).then(|| {
			format!(
				"the game's window could not be drawn to: the system refused its surface for {} s \
				({} attempts; the last, {})",
				GIVE_UP.as_secs(),
				self.attempts,
				self.last
			)
		})
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_first_failure_and_the_recovery_are_told_once() {
		let start = Instant::now();
		let mut trouble = None;
		let first = SurfaceTrouble::failed(&mut trouble, "configure", "Invalid surface", start);
		assert!(matches!(first, Outcome::Warn(ref s) if s.contains("Invalid surface")), "{first:?}");
		let later = start + Duration::from_millis(16);
		assert_eq!(SurfaceTrouble::failed(&mut trouble, "configure", "Invalid surface", later), Outcome::Fine);
		let done = SurfaceTrouble::configured(&mut trouble, start + Duration::from_millis(40));
		assert_eq!(done, Outcome::Recovered("the window's surface is configured again, after 2 failed attempts in 40 ms".into()));
		assert!(trouble.is_none());
		assert_eq!(SurfaceTrouble::configured(&mut trouble, start), Outcome::Fine);
	}

	#[test]
	fn it_gives_up_after_a_while_with_the_last_error() {
		let start = Instant::now();
		let mut trouble = None;
		SurfaceTrouble::failed(&mut trouble, "configure", "a", start);
		SurfaceTrouble::failed(&mut trouble, "present", "b", start + Duration::from_secs(1));
		let t = trouble.as_ref().unwrap();
		assert_eq!(t.given_up(start + GIVE_UP - Duration::from_millis(1)), None);
		let error = t.given_up(start + GIVE_UP).expect("given up");
		assert!(error.contains("2 attempts") && error.contains("present: b"), "{error}");
	}
}
