//! The processes bb starts end with it. CEF runs `bb.exe` again for its child processes
//! (the page's renderer, the GPU process, utilities, …). They end when bb shuts CEF
//! down, and on their own once they notice the game's process is gone, but that's up
//! to each of them, in its own time: a child stuck (in a driver, say) outlives bb. One
//! that does keeps `cef.log` open, which Chromium opens without letting it be deleted
//! (under Wine, `del cef.log` while CEF runs: "Sharing violation"), so the next start
//! can't archive it (as a log showed: "The process cannot access the file
//! because it is being used by another process"), and its memory stays taken.
//!
//! So bb puts itself in a job object that ends its processes when its last handle
//! closes, which is when bb's process ends, however it ends (a quit, a panic, a native
//! crash, Task Manager). The processes it starts join the job. The ones that must
//! outlive it leave it ([`BREAKAWAY`]): the browser that opens a page's link.

use std::os::windows::process::CommandExt;
use std::process::Command;

use windows::Win32::System::JobObjects::{
	AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_BREAKAWAY_OK, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
	JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation, SetInformationJobObject,
};
use windows::Win32::System::Threading::{CREATE_BREAKAWAY_FROM_JOB, GetCurrentProcess};
use windows::core::PCWSTR;

/// `CreateProcess`'s flag for a child that leaves the job.
pub const BREAKAWAY: u32 = CREATE_BREAKAWAY_FROM_JOB.0;

/// Put this process in a job that ends every process in it when this one ends. Call
/// in the game's own process before it starts any (CEF, a relaunch). The job's handle
/// is never closed: the system closes it as the process ends, which ends the job.
pub fn end_children_with_bb() -> Result<(), String> {
	// SAFETY: the job is a new object of this process's; the information passed is the
	// structure the class names, of its size.
	unsafe {
		let job = CreateJobObjectW(None, PCWSTR::null()).map_err(|e| format!("a job for bb's processes: {e}"))?;
		let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
		limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_BREAKAWAY_OK;
		SetInformationJobObject(
			job,
			JobObjectExtendedLimitInformation,
			(&raw const limits).cast(),
			size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
		)
		.map_err(|e| format!("the job's limits: {e}"))?;
		// Nested in any job bb was started in (Windows 8 and later).
		AssignProcessToJobObject(job, GetCurrentProcess()).map_err(|e| format!("bb into its job: {e}"))?;
	}
	Ok(())
}

/// Start `command` outside bb's job, so it outlives bb (it may start a browser that
/// stays). A job bb was started in that doesn't let processes leave it refuses
/// that: then it starts in the job.
pub fn spawn_outside(command: &mut Command) -> std::io::Result<std::process::Child> {
	command.creation_flags(BREAKAWAY).spawn().or_else(|_| command.creation_flags(0).spawn())
}

#[cfg(test)]
mod tests;
