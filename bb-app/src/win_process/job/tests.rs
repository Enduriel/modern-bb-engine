//! A process bb started ends when bb does, however bb ends: this test binary runs itself
//! as a stand-in for bb (`ROLE` = `bb`), which starts a long-lived child (`ROLE` =
//! `child`, a stand-in for CEF's) and then dies at once (an abort, as a crash would end
//! it). Without the job the child lives on.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::time::Duration;

use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{
	OpenProcess, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, TerminateProcess, WaitForSingleObject,
};

const ROLE: &str = "BB_TEST_JOB_ROLE";
const HELPER: &str = "win_process::job::tests::helper";

/// The stand-ins' side (does nothing in a normal test run).
#[test]
fn helper() {
	match std::env::var(ROLE).as_deref() {
		Ok("child") => std::thread::sleep(Duration::from_secs(60)),
		Ok(role @ ("bb" | "bb-without-job")) => {
			if role == "bb" {
				super::end_children_with_bb().unwrap();
			}
			let exe = std::env::current_exe().unwrap();
			let child = Command::new(exe)
				.args([HELPER, "--exact", "--nocapture"])
				.env(ROLE, "child")
				.stdout(Stdio::null())
				.spawn()
				.unwrap();
			println!("CHILD {}", child.id());
			std::process::abort();
		}
		_ => {}
	}
}

/// Run a stand-in bb (`role`) and wait up to `wait` for the child it started to end;
/// whether it ended (a child still running is ended here).
fn child_ends(role: &str, wait: Duration) -> bool {
	let exe = std::env::current_exe().unwrap();
	let mut bb = Command::new(exe)
		.args([HELPER, "--exact", "--nocapture"])
		.env(ROLE, role)
		.stdout(Stdio::piped())
		.spawn()
		.unwrap();
	let pid: u32 = BufReader::new(bb.stdout.take().unwrap())
		.lines()
		.map_while(Result::ok)
		.find_map(|line| line.strip_prefix("CHILD ").map(|p| p.trim().parse().unwrap()))
		.expect("the stand-in bb names its child");
	let _ = bb.wait();
	// SAFETY: a handle of the child's, waited on and closed here.
	unsafe {
		let Ok(child) = OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_TERMINATE, false, pid) else { return true };
		let ended = WaitForSingleObject(child, wait.as_millis() as u32) == WAIT_OBJECT_0;
		if !ended {
			let _ = TerminateProcess(child, 1);
		}
		let _ = CloseHandle(child);
		ended
	}
}

#[test]
fn a_crashed_bbs_processes_end_with_it() {
	assert!(child_ends("bb", Duration::from_secs(10)), "the child outlived the stand-in bb");
	// The check itself: without the job, the child does outlive it.
	assert!(!child_ends("bb-without-job", Duration::from_secs(2)), "the child ended without the job");
}
