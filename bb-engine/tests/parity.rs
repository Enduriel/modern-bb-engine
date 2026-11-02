//! The ground-truth parity suite. Runs every probe in `parity/probes/` in our
//! engine, exactly as `parity/capture.py` runs them in the original game (grouped by
//! mode and seed, concatenated in name order, each in its own function behind a
//! `PROBE_SECTION` marker), and compares each `PROBE key = value` line with the golden
//! captured from the original (`parity/goldens/`).
//!
//! A probe with a `// compare: subset` line is a coverage check: only the original's
//! lines must appear in ours.
//!
//! A probe compares what gets logged with a log window: after a `PROBE_LOG <key>` line,
//! every row logged up to the next `PROBE` line (by the scripts or the engine, any tag
//! or level) must match the original's, in count and order (see `log_windows`).
//!
//! A known, accepted difference is listed in `parity/divergences.txt` as
//! `probe | key | reason`; one that starts matching again fails the test, so the list
//! stays honest. A probe without a golden is reported (capture it) but does not fail.
//! `PARITY_DUMP=<dir>` also writes our engine's output per probe there, for review.
//! Skips if the Steam install is missing.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use bb_engine::{harness, tactical_player};
use squirrel_bb::html_log;

#[path = "support/parity.rs"]
mod common;

use common::{Report, compare, entries, finish, golden_lines, load_divergences, normalize, parity_dir};

struct Probe {
	name: String,
	mode: String,
	seed: Option<String>,
	/// `// compare: subset`: only the original's lines must be in ours.
	subset: bool,
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
			let subset = body.lines().any(|l| l.trim() == "// compare: subset");
			let name = path
				.strip_prefix(root)
				.unwrap()
				.with_extension("")
				.to_string_lossy()
				.replace('\\', "/");
			Probe { name, mode, seed, subset, body }
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

const LOG_START: &str = "PROBE_LOG ";

/// The rows' PROBE lines, with each log window expanded: a row `PROBE_LOG <key>` opens
/// one and the next row whose text starts with `PROBE` closes it; the rows in between
/// become `PROBE <key> log = <n> rows` and `PROBE <key> log <i> = <class> <tag> <text>`.
/// Must match `parity/probes.py::log_windows` (and `capture.py`, which keeps the PROBE
/// rows and puts each window in place of its `PROBE_LOG` line).
fn log_windows(rows: &[(String, String, String)]) -> Vec<String> {
	fn close(out: &mut Vec<String>, key: &str, inside: &[String]) {
		out.push(format!("PROBE {key} log = {} rows", inside.len()));
		out.extend(inside.iter().enumerate().map(|(i, r)| format!("PROBE {key} log {} = {r}", i + 1)));
	}
	let mut out = Vec::new();
	let mut window: Option<(String, Vec<String>)> = None;
	for (class, tag, text) in rows {
		let text = normalize(text);
		if text.starts_with("PROBE") {
			if let Some((key, inside)) = window.take() {
				close(&mut out, &key, &inside);
			}
			match text.strip_prefix(LOG_START) {
				Some(key) => window = Some((key.to_string(), Vec::new())),
				None => out.push(text),
			}
		} else if let Some((_, inside)) = window.as_mut() {
			inside.push(normalize(&format!("{class} {tag} {text}")));
		}
	}
	if let Some((key, inside)) = window {
		close(&mut out, &key, &inside);
	}
	out
}

/// Run one (mode, seed) group in our engine; the PROBE lines in order.
fn run_group(mode: &str, seed: Option<&str>, probes: &[&Probe]) -> Vec<String> {
	let mut engine = harness::open_install(Path::new(harness::STEAM_DATA_DIR)).expect("open install");
	// The rows come through the tap; the terminal stays quiet.
	engine.set_log_handler(|_, _| {});
	// Every row log.html would get (the original's runner reads the same rows).
	let rows = Arc::new(Mutex::new(Vec::new()));
	let sink = rows.clone();
	html_log::set_tap(Some(Box::new(move |class: html_log::Class, tag: &str, text: &[u8]| {
		let row = (class.name().to_string(), tag.to_string(), String::from_utf8_lossy(text).into_owned());
		sink.lock().unwrap().push(row);
	})));
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
	// As the probe harness: a probe that observes the game over time (the group's last)
	// sets `BbProbe.DeferDone` and calls `BbProbe.done()` from a scheduled event.
	engine
		.run_script("::BbProbe <- { DeferDone = false, Done = false, function done() { this.Done = true; } };", "probe harness")
		.expect("the probe harness");
	// Probes are game-style code: single-precision floats, like the original.
	let vm = engine.vm();
	let closure = vm.compile(&combined_source(probes), "parity").expect("the probes compile");
	closure.set_f32_math(vm, true);
	closure.call(vm, ()).expect("each probe catches its own errors");
	let deferred = "(::BbProbe.DeferDone && !::BbProbe.Done) ? 1 : 0";
	let mut frames = 0;
	while tactical_player::eval_i64(&engine, deferred).expect("the probe harness") == Some(1) {
		// The original's runner gives a probe 360 s.
		assert!(frames < 360 * 60, "a deferred probe never called BbProbe.done()");
		harness::run_frame(&engine, 1000.0 / 60.0).expect("a frame");
		frames += 1;
	}
	html_log::set_tap(None);
	let rows = rows.lock().unwrap();
	log_windows(&rows)
}

#[test]
fn probes_match_the_original() {
	if !Path::new(harness::STEAM_DATA_DIR).exists() {
		bb_engine::harness::skip(format!("no install at {}", harness::STEAM_DATA_DIR));
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
			let Some(golden) = golden_lines(&golden_path) else {
				report.uncaptured.push(p.name.clone());
				continue;
			};
			let ours = sections.get(&p.name).cloned().unwrap_or_default();
			compare(&p.name, &entries(&golden), &entries(&ours), &allowed, p.subset, &mut report);
		}
	}
	finish(report, &allowed);
}
