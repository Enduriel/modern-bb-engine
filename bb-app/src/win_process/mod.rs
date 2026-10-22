//! bb's process on Windows, where it is a GUI program (`windows_subsystem`), so that a
//! player who double-clicks it gets no console window behind the game:
//! - [`console`]: run from a console (cmd, PowerShell, a terminal), its output goes to
//!   that console, and output redirected to a file or a pipe stays there.
//! - [`job`]: the processes it starts (CEF's, which run `bb.exe` again) end when it
//!   ends, however it ends.
//! - [`crash`]: a native crash (an exception nothing handles) leaves a line in log.html
//!   and tells the player, as a panic does.

pub mod console;
pub mod crash;
pub mod job;
