//! What the parity tests share: reading goldens and `parity/divergences.txt`, and
//! comparing `PROBE key = value` lines (see `parity/README.md`).

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The repo's `parity/` directory.
pub fn parity_dir() -> PathBuf {
	Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parity")
}

/// As the probe runner prints log rows: whitespace runs collapsed, trimmed.
pub fn normalize(line: &str) -> String {
	line.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `PROBE key = value` lines as an ordered key → value list (a section failure keeps its
/// whole text as the key).
pub fn entries(lines: &[String]) -> Vec<(String, String)> {
	lines
		.iter()
		.map(|l| {
			let l = l.strip_prefix("PROBE ").unwrap_or(l);
			match l.split_once(" = ") {
				Some((k, v)) => (k.to_string(), v.to_string()),
				None => (l.to_string(), String::new()),
			}
		})
		.collect()
}

#[derive(Default)]
pub struct Report {
	pub mismatches: Vec<String>,
	pub stale: Vec<String>,
	pub uncaptured: Vec<String>,
	pub checked: usize,
	pub diverged: usize,
}

pub fn load_divergences() -> BTreeMap<(String, String), String> {
	let text = std::fs::read_to_string(parity_dir().join("divergences.txt")).unwrap_or_default();
	text.lines()
		.filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
		.map(|l| {
			// " | " with spaces: a probe key may contain a bare '|'.
			let parts: Vec<&str> = l.splitn(3, " | ").map(str::trim).collect();
			assert_eq!(parts.len(), 3, "divergences.txt: expected 'probe | key | reason': {l}");
			((parts[0].to_string(), parts[1].to_string()), parts[2].to_string())
		})
		.collect()
}

pub fn compare(probe: &str, golden: &[(String, String)], ours: &[(String, String)], allowed: &BTreeMap<(String, String), String>, report: &mut Report) {
	let ours_map: BTreeMap<&str, &str> = ours.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
	let golden_map: BTreeMap<&str, &str> = golden.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
	for (key, want) in golden {
		report.checked += 1;
		let got = ours_map.get(key.as_str()).copied();
		let known = allowed.contains_key(&(probe.to_string(), key.clone()));
		match (got == Some(want.as_str()), known) {
			(true, false) => {}
			(true, true) => report.stale.push(format!("{probe} | {key}: listed as a divergence but now matches")),
			(false, true) => report.diverged += 1,
			(false, false) => report.mismatches.push(format!(
				"{probe} | {key}\n    original: {want}\n    ours:     {}",
				got.unwrap_or("<missing>")
			)),
		}
	}
	for (key, value) in ours {
		if !golden_map.contains_key(key.as_str()) {
			report.mismatches.push(format!("{probe} | {key}\n    original: <missing>\n    ours:     {value}"));
		}
	}
}

/// A golden's lines, without its `#` comments, normalized.
pub fn golden_lines(path: &Path) -> Option<Vec<String>> {
	let text = std::fs::read_to_string(path).ok()?;
	Some(text.lines().filter(|l| !l.starts_with('#')).map(normalize).collect())
}

/// Print the summary and fail on any mismatch or stale divergence.
pub fn finish(report: Report) {
	eprintln!(
		"parity: {} checks, {} known divergences, {} mismatches, {} uncaptured probes",
		report.checked,
		report.diverged,
		report.mismatches.len(),
		report.uncaptured.len()
	);
	for name in &report.uncaptured {
		eprintln!("  no golden for {name} (run parity/capture.py)");
	}
	let problems: Vec<String> = report.mismatches.into_iter().chain(report.stale).collect();
	assert!(problems.is_empty(), "parity differences:\n{}", problems.join("\n"));
}
