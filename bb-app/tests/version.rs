//! `bb --version`: one line, the version, target and UI, and nothing started.

use std::process::Command;

#[test]
fn version_names_the_build_and_exits() {
	let output = Command::new(env!("CARGO_BIN_EXE_bb")).arg("--version").output().unwrap();
	assert!(output.status.success(), "{output:?}");
	let stdout = String::from_utf8(output.stdout).unwrap();
	// The `bb` beside this test may be the UI build's or the plain one's (test_all.py
	// builds both into one folder), so either form is right.
	let plain = format!("bb {} ({})\n", env!("CARGO_PKG_VERSION"), env!("BB_TARGET"));
	let ui = format!("bb {} ({}, ui)\n", env!("CARGO_PKG_VERSION"), env!("BB_TARGET"));
	assert!(stdout == plain || stdout == ui, "{stdout:?}");
}
