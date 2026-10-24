//! What asks for a snapshot: the player's hotkey (Ctrl+Shift+F11, every system), and on
//! Linux and macOS the signal `SIGUSR1`, so one can be taken from outside the game
//! without touching its input (`kill -USR1 <pid>`).
//!
//! The signal's handler only sets a flag (all a handler may safely do); the game's
//! thread takes the snapshot at its next frame ([`take_signal`]). Every process of bb's
//! ignores the signal from its start ([`ignore_signal`]): CEF runs bb again for its
//! child processes, which would otherwise end on it (its default), and the game's own
//! process could be hit before it listens. The game's process listens once it runs
//! ([`listen_for_signal`]).

use std::sync::atomic::{AtomicBool, Ordering};

use winit::keyboard::{KeyCode, ModifiersState};

/// What asked for a snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
	Hotkey,
	Signal,
}

impl Trigger {
	pub fn name(self) -> &'static str {
		match self {
			Trigger::Hotkey => "the hotkey (Ctrl+Shift+F11)",
			Trigger::Signal => "SIGUSR1",
		}
	}
}

/// Whether `key` pressed with `modifiers` held asks for a snapshot: Ctrl+Shift+F11.
/// Neither vanilla nor the mods surveyed bind it; vanilla binds F11 alone in developer
/// mode (the tooltips' delay, whatever the modifiers), so the game never sees this key.
pub fn is_hotkey(key: KeyCode, modifiers: ModifiersState) -> bool {
	key == KeyCode::F11 && modifiers == ModifiersState::CONTROL | ModifiersState::SHIFT
}

/// Set by the signal's handler.
static SIGNALLED: AtomicBool = AtomicBool::new(false);

/// Whether the signal came since the last call.
pub fn take_signal() -> bool {
	SIGNALLED.swap(false, Ordering::Relaxed)
}

#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "macos"))]
extern "C" fn on_signal(_signal: libc::c_int) {
	SIGNALLED.store(true, Ordering::Relaxed);
}

/// Ignore the signal in this process (and the processes it starts, which inherit that);
/// see the module's notes. Call first thing in `main`.
pub fn ignore_signal() {
	#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "macos"))]
	// SAFETY: ignoring a signal changes no memory; SIGUSR1 is ours alone.
	unsafe {
		libc::signal(libc::SIGUSR1, libc::SIG_IGN);
	}
}

/// Take snapshots on the signal from now on, in the game's own process. Interrupted
/// system calls restart (`SA_RESTART`): the signal is harmless to every thread. Where
/// there is no such signal (Windows), the hotkey alone.
pub fn listen_for_signal() -> Result<(), String> {
	#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "macos"))]
	// SAFETY: the handler only stores to an atomic, which is async-signal-safe; the
	// action is zeroed but for the fields set.
	unsafe {
		let mut action: libc::sigaction = std::mem::zeroed();
		action.sa_sigaction = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
		action.sa_flags = libc::SA_RESTART;
		libc::sigemptyset(&mut action.sa_mask);
		if libc::sigaction(libc::SIGUSR1, &action, std::ptr::null_mut()) != 0 {
			return Err(std::io::Error::last_os_error().to_string());
		}
	}
	Ok(())
}

/// Whether this system has the signal.
pub const HAS_SIGNAL: bool = cfg!(any(target_os = "linux", target_os = "freebsd", target_os = "macos"));

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_hotkey_is_ctrl_shift_f11_alone() {
		let ctrl_shift = ModifiersState::CONTROL | ModifiersState::SHIFT;
		assert!(is_hotkey(KeyCode::F11, ctrl_shift));
		assert!(!is_hotkey(KeyCode::F11, ModifiersState::empty()), "vanilla's developer key");
		assert!(!is_hotkey(KeyCode::F11, ModifiersState::CONTROL));
		assert!(!is_hotkey(KeyCode::F11, ctrl_shift | ModifiersState::ALT));
		assert!(!is_hotkey(KeyCode::KeyF, ctrl_shift), "the FPS display's");
	}

	#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "macos"))]
	#[test]
	fn the_signal_sets_the_flag() {
		listen_for_signal().expect("listen");
		// SAFETY: raising a signal whose handler only sets a flag.
		unsafe {
			libc::raise(libc::SIGUSR1);
		}
		assert!(take_signal());
		assert!(!take_signal(), "taken once");
	}
}
