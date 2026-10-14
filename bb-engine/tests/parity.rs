//! The ground-truth parity suite. Runs every probe in `parity/probes/` in our
//! engine, exactly as `parity/capture.py` runs them in the original game (grouped by
//! mode and seed, concatenated in name order, each in its own function behind a
//! `PROBE_SECTION` marker), and compares each `PROBE key = value` line with the golden
//! captured from the original (`parity/goldens/`).
//!
//! A known, accepted difference is listed in `parity/divergences.txt` as
//! `probe | key | reason`; one that starts matching again fails the test, so the list
//! stays honest. A probe without a golden is reported (capture it) but does not fail.
//! `PARITY_DUMP=<dir>` also writes our engine's output per probe there, for review.
//! Skips if the Steam install is missing.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::cell::RefCell;

use bb_engine::harness;
use squirrel_bb::LogLevel;

fn parity_dir() -> PathBuf {
	Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parity")
}

struct Probe {
	name: String,
	mode: String,
	seed: Option<String>,
	body: String,
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
	for entry in std::fs::read_dir(dir).unwrap().flatten() {
		let path = entry.path();
		if path.is_dir() {
			collect(&path, out);
		} else if path.extension().is_some_and(|e| e == "nut") {
			out.push(path);
		}
	}
}

/// The probes, sorted by name (the order `capture.py` uses).
fn load_probes(root: &Path) -> Vec<Probe> {
	let mut paths = Vec::new();
	collect(root, &mut paths);
	let mut probes: Vec<Probe> = paths
		.into_iter()
		.map(|path| {
			let body = std::fs::read_to_string(&path).unwrap();
			let header = body
				.lines()
				.find_map(|l| l.strip_prefix("// mode: "))
				.unwrap_or_else(|| panic!("{}: missing '// mode:' header", path.display()));
			let mut words = header.split_whitespace();
			let mode = words.next().unwrap().to_string();
			let seed = words.find_map(|w| w.strip_prefix("seed=")).map(str::to_string);
			let name = path
				.strip_prefix(root)
				.unwrap()
				.with_extension("")
				.to_string_lossy()
				.replace('\\', "/");
			Probe { name, mode, seed, body }
		})
		.collect();
	probes.sort_by(|a, b| a.name.cmp(&b.name));
	probes
}

/// Must match `parity/probes.py::combined_source` byte for byte.
fn combined_source(probes: &[&Probe]) -> String {
	let mut out = String::new();
	for p in probes {
		out += &format!("::logInfo(\"PROBE_SECTION {}\");\n", p.name);
		out += "try { (function() {\n";
		out += &p.body;
		out += "\n}).call(::getroottable()); } catch (e) { ::logInfo(\"PROBE_SECTION_FAIL \" + e); }\n";
	}
	out
}

/// As the probe runner prints log rows: whitespace runs collapsed, trimmed.
fn normalize(line: &str) -> String {
	line.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn split_sections(lines: &[String]) -> BTreeMap<String, Vec<String>> {
	let mut sections = BTreeMap::new();
	let mut current: Option<String> = None;
	for line in lines {
		if let Some(name) = line.strip_prefix("PROBE_SECTION ") {
			current = Some(name.trim().to_string());
			sections.insert(name.trim().to_string(), Vec::new());
		} else if let Some(name) = &current
			&& (line.starts_with("PROBE ") || line.starts_with("PROBE_SECTION_FAIL"))
		{
			sections.get_mut(name).unwrap().push(line.clone());
		}
	}
	sections
}

/// `PROBE key = value` lines as an ordered key → value list (a section failure keeps its
/// whole text as the key).
fn entries(lines: &[String]) -> Vec<(String, String)> {
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

/// Run one (mode, seed) group in our engine; the PROBE lines in order.
fn run_group(mode: &str, seed: Option<&str>, probes: &[&Probe]) -> Vec<String> {
	let mut engine = harness::open_install(Path::new(harness::STEAM_DATA_DIR)).expect("open install");
	let lines = Rc::new(RefCell::new(Vec::new()));
	let sink = lines.clone();
	engine.set_log_handler(move |level, message| {
		if level == LogLevel::Info && message.starts_with("PROBE") {
			sink.borrow_mut().push(normalize(message));
		}
	});
	harness::boot_menu(&engine).expect("boot the menu");
	if mode == "campaign" {
		let seed = seed.unwrap_or("probe");
		harness::prepare_new_campaign(&engine, seed).expect("prepare the campaign");
		// The probe harness starts campaigns with the first player banner.
		engine
			.run_script("::__bootWorldState.m.CampaignSettings.Banner = ::Const.PlayerBanners[0];", "banner")
			.expect("set the banner");
		harness::start_new_campaign(&engine).expect("startNewCampaign");
	}
	// Probes are game-style code: single-precision floats, like the original.
	let vm = engine.vm();
	let closure = vm.compile(&combined_source(probes), "parity").expect("the probes compile");
	closure.set_f32_math(vm, true);
	closure.call(vm, ()).expect("each probe catches its own errors");
	let out = lines.borrow().clone();
	out
}

#[derive(Default)]
struct Report {
	mismatches: Vec<String>,
	stale: Vec<String>,
	uncaptured: Vec<String>,
	checked: usize,
	diverged: usize,
}

fn load_divergences() -> BTreeMap<(String, String), String> {
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

fn compare(probe: &str, golden: &[(String, String)], ours: &[(String, String)], allowed: &BTreeMap<(String, String), String>, report: &mut Report) {
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

#[test]
fn probes_match_the_original() {
	if !Path::new(harness::STEAM_DATA_DIR).exists() {
		eprintln!("skipping: no install at {}", harness::STEAM_DATA_DIR);
		return;
	}
	let probes = load_probes(&parity_dir().join("probes"));
	let allowed = load_divergences();
	let mut groups: BTreeMap<(String, Option<String>), Vec<&Probe>> = BTreeMap::new();
	for p in &probes {
		groups.entry((p.mode.clone(), p.seed.clone())).or_default().push(p);
	}
	let mut report = Report::default();
	for ((mode, seed), group) in &groups {
		let sections = split_sections(&run_group(mode, seed.as_deref(), group));
		if let Some(dump) = std::env::var_os("PARITY_DUMP") {
			for (name, lines) in &sections {
				let path = Path::new(&dump).join(format!("{name}.txt"));
				std::fs::create_dir_all(path.parent().unwrap()).unwrap();
				std::fs::write(path, lines.join("\n") + "\n").unwrap();
			}
		}
		for p in group {
			let golden_path = parity_dir().join("goldens").join(format!("{}.txt", p.name));
			let Ok(golden) = std::fs::read_to_string(&golden_path) else {
				report.uncaptured.push(p.name.clone());
				continue;
			};
			let golden: Vec<String> = golden.lines().filter(|l| !l.starts_with('#')).map(normalize).collect();
			let ours = sections.get(&p.name).cloned().unwrap_or_default();
			compare(&p.name, &entries(&golden), &entries(&ours), &allowed, &mut report);
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
