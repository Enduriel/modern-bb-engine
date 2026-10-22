//! What the parity tests share: reading goldens and `parity/divergences.txt`, and
//! comparing `PROBE key = value` lines (see `parity/README.md`).

#![allow(dead_code)]

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// The repo's `parity/` directory.
pub fn parity_dir() -> PathBuf {
	Path::new(env!("CARGO_MANIFEST_DIR")).join("../parity")
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
	/// The probes compared, for judging which globs went unused.
	pub compared: BTreeSet<String>,
}

/// The known, accepted differences: `parity/divergences.txt` (deliberate or unmatchable)
/// and `parity/missing_natives.txt` (natives the original binds that we don't yet), as
/// `probe | key | reason`. A key with one `*` matches every key with that prefix and suffix.
#[derive(Default)]
pub struct Allowed {
	exact: BTreeMap<(String, String), String>,
	globs: Vec<(String, String, String)>,
	/// The globs that excused a difference, by index.
	used: RefCell<BTreeSet<usize>>,
}

impl Allowed {
	fn matching_glob(&self, probe: &str, key: &str) -> Option<usize> {
		self.globs.iter().position(|(p, prefix, suffix)| {
			p == probe && key.len() >= prefix.len() + suffix.len() && key.starts_with(prefix.as_str()) && key.ends_with(suffix.as_str())
		})
	}
}

pub fn load_divergences() -> Allowed {
	let mut allowed = Allowed::default();
	for file in ["divergences.txt", "missing_natives.txt"] {
		let text = std::fs::read_to_string(parity_dir().join(file)).unwrap_or_default();
		for l in text.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#')) {
			// " | " with spaces: a probe key may contain a bare '|'.
			let parts: Vec<&str> = l.splitn(3, " | ").map(str::trim).collect();
			assert_eq!(parts.len(), 3, "{file}: expected 'probe | key | reason': {l}");
			match parts[1].split_once('*') {
				Some((prefix, suffix)) => allowed.globs.push((parts[0].into(), prefix.into(), suffix.into())),
				None => {
					allowed.exact.insert((parts[0].into(), parts[1].into()), parts[2].into());
				}
			}
		}
	}
	allowed
}

/// Compare one probe's lines. With `subset`, lines only ours has are fine (a coverage
/// probe: everything the original has must exist in ours).
pub fn compare(
	probe: &str,
	golden: &[(String, String)],
	ours: &[(String, String)],
	allowed: &Allowed,
	subset: bool,
	report: &mut Report,
) {
	report.compared.insert(probe.to_string());
	let ours_map: BTreeMap<&str, &str> = ours.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
	let golden_map: BTreeMap<&str, &str> = golden.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
	for (key, want) in golden {
		report.checked += 1;
		let got = ours_map.get(key.as_str()).copied();
		let listed = allowed.exact.contains_key(&(probe.to_string(), key.clone()));
		let matches = got == Some(want.as_str());
		let glob = if matches || listed { None } else { allowed.matching_glob(probe, key) };
		if let Some(i) = glob {
			allowed.used.borrow_mut().insert(i);
		}
		match (matches, listed || glob.is_some()) {
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
		if !subset && !golden_map.contains_key(key.as_str()) {
			report.mismatches.push(format!("{probe} | {key}\n    original: <missing>\n    ours:     {value}"));
		}
	}
}

/// A golden's lines, without its `#` comments, normalized.
pub fn golden_lines(path: &Path) -> Option<Vec<String>> {
	let text = std::fs::read_to_string(path).ok()?;
	Some(text.lines().filter(|l| !l.starts_with('#')).map(normalize).collect())
}

/// Print the summary and fail on any mismatch or stale divergence (a listed key that now
/// matches, or a glob that excused nothing).
pub fn finish(mut report: Report, allowed: &Allowed) {
	let used = allowed.used.borrow();
	for (i, (probe, prefix, suffix)) in allowed.globs.iter().enumerate() {
		if report.compared.contains(probe) && !used.contains(&i) {
			report.stale.push(format!("{probe} | {prefix}*{suffix}: listed, but no difference matches it"));
		}
	}
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
