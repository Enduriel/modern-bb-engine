use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use super::*;

/// An empty folder of the test's own.
fn folder(name: &str) -> PathBuf {
	let dir = std::env::temp_dir().join(format!("bb-log-files-{}-{name}", std::process::id()));
	let _ = fs::remove_dir_all(&dir);
	fs::create_dir_all(&dir).unwrap();
	dir
}

/// The zip's entries: (name, bytes).
fn entries(zip: &Path) -> Vec<(String, Vec<u8>)> {
	let mut zip = zip::ZipArchive::new(fs::File::open(zip).unwrap()).unwrap();
	(0..zip.len())
		.map(|i| {
			let mut file = zip.by_index(i).unwrap();
			let mut bytes = Vec::new();
			file.read_to_end(&mut bytes).unwrap();
			(file.name().to_string(), bytes)
		})
		.collect()
}

fn zips(dir: &Path) -> Vec<String> {
	let mut names: Vec<String> = fs::read_dir(dir.join("logs"))
		.map(|r| r.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
		.unwrap_or_default();
	names.sort();
	names
}

#[test]
fn a_first_start_has_nothing_to_keep() {
	let dir = folder("first");
	let files = LogFiles::start(&dir).unwrap();
	assert_eq!(files.html, dir.join("log.html"));
	assert_eq!(files.cef, dir.join("cef.log"));
	assert!(files.archived.is_empty());
	// The lock records the session's start.
	assert!(fs::read_to_string(dir.join("log.lock")).unwrap().ends_with('Z'));
	drop(files);
	fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_last_sessions_logs_are_zipped_each_alone_and_intact() {
	let dir = folder("previous");
	let log = b"<html><head><title>Battle Brothers 1.5.2.3</title>\xe9 rows".repeat(500);
	fs::write(dir.join("log.html"), &log).unwrap();
	fs::write(dir.join("cef.log"), b"[1020/101546.123:WARNING:x] cef").unwrap();
	fs::write(dir.join("log.lock"), "2026-10-19T20-00-01.250Z").unwrap();
	let files = LogFiles::start(&dir).unwrap();
	assert_eq!(zips(&dir), ["cef-2026-10-19T20-00-01.250Z.zip", "log-2026-10-19T20-00-01.250Z.zip"]);
	assert_eq!(entries(&dir.join("logs/log-2026-10-19T20-00-01.250Z.zip")), [("log.html".to_string(), log.clone())]);
	assert_eq!(
		entries(&dir.join("logs/cef-2026-10-19T20-00-01.250Z.zip")),
		[("cef.log".to_string(), b"[1020/101546.123:WARNING:x] cef".to_vec())]
	);
	assert!(!dir.join("log.html").exists() && !dir.join("cef.log").exists());
	assert_eq!(files.archived.len(), 2);
	// Compressed: the zip is a fraction of the log.
	assert!(fs::metadata(dir.join("logs/log-2026-10-19T20-00-01.250Z.zip")).unwrap().len() < log.len() as u64 / 10);
	drop(files);
	fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_crashed_sessions_log_is_kept_under_its_start() {
	let dir = folder("crash");
	let first = LogFiles::start(&dir).unwrap();
	let started = fs::read_to_string(dir.join("log.lock")).unwrap();
	fs::write(&first.html, "rows, and no page end").unwrap();
	// The process dies: nothing tidies up, the lock goes with it.
	drop(first);
	let second = LogFiles::start(&dir).unwrap();
	assert_eq!(second.html, dir.join("log.html"));
	assert_eq!(zips(&dir), [format!("log-{started}.zip")]);
	assert_eq!(entries(&dir.join(format!("logs/log-{started}.zip")))[0].1, b"rows, and no page end");
	drop(second);
	fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn two_sessions_at_once_log_apart() {
	let dir = folder("two");
	let a = LogFiles::start(&dir).unwrap();
	fs::write(&a.html, "a's").unwrap();
	let b = LogFiles::start(&dir).unwrap();
	assert_eq!(b.html, dir.join("log.2.html"));
	assert_eq!(b.cef, dir.join("cef.2.log"));
	fs::write(&b.html, "b's").unwrap();
	// A's log is untouched while it runs.
	assert_eq!(fs::read_to_string(&a.html).unwrap(), "a's");
	assert!(zips(&dir).is_empty());
	drop((a, b));
	// The next start takes the first slot and keeps both.
	let c = LogFiles::start(&dir).unwrap();
	assert_eq!(c.html, dir.join("log.html"));
	assert_eq!(c.archived.len(), 2);
	let mut kept: Vec<Vec<u8>> = c.archived.iter().map(|z| entries(z).remove(0).1).collect();
	kept.sort();
	assert_eq!(kept, [b"a's".to_vec(), b"b's".to_vec()]);
	assert!(!dir.join("log.2.html").exists());
	drop(c);
	fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_archive_never_replaces_another() {
	let dir = folder("names");
	fs::create_dir_all(dir.join("logs")).unwrap();
	fs::write(dir.join("logs/log-2026-10-19T20-00-01.250Z.zip"), "earlier").unwrap();
	fs::write(dir.join("log.html"), "later").unwrap();
	fs::write(dir.join("log.lock"), "2026-10-19T20-00-01.250Z").unwrap();
	let files = LogFiles::start(&dir).unwrap();
	assert_eq!(zips(&dir), ["log-2026-10-19T20-00-01.250Z-2.zip", "log-2026-10-19T20-00-01.250Z.zip"]);
	assert_eq!(fs::read_to_string(dir.join("logs/log-2026-10-19T20-00-01.250Z.zip")).unwrap(), "earlier");
	drop(files);
	fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_empty_log_is_not_kept() {
	let dir = folder("empty");
	fs::write(dir.join("log.html"), "").unwrap();
	let files = LogFiles::start(&dir).unwrap();
	assert!(files.archived.is_empty() && zips(&dir).is_empty());
	drop(files);
	fs::remove_dir_all(&dir).unwrap();
}

/// A zip that can't be written (here: `logs/` read-only) leaves the old log in place,
/// whole; the session logs beside it, and a later start keeps it.
#[cfg(unix)]
#[test]
fn a_failed_archive_keeps_the_old_log() {
	use std::os::unix::fs::PermissionsExt;
	let dir = folder("fail");
	fs::write(dir.join("log.html"), "the old log").unwrap();
	fs::create_dir_all(dir.join("logs")).unwrap();
	fs::set_permissions(dir.join("logs"), fs::Permissions::from_mode(0o555)).unwrap();
	let files = LogFiles::start(&dir).unwrap();
	fs::set_permissions(dir.join("logs"), fs::Permissions::from_mode(0o755)).unwrap();
	assert_eq!(fs::read_to_string(dir.join("log.html")).unwrap(), "the old log");
	assert_eq!(files.html, dir.join("log.2.html"));
	assert_eq!(files.problems.len(), 1, "{:?}", files.problems);
	assert!(zips(&dir).is_empty(), "no temporary file left: {:?}", zips(&dir));
	fs::write(&files.html, "the new log").unwrap();
	drop(files);
	let next = LogFiles::start(&dir).unwrap();
	assert_eq!(next.html, dir.join("log.html"));
	let mut kept: Vec<Vec<u8>> = next.archived.iter().map(|z| entries(z).remove(0).1).collect();
	kept.sort();
	assert_eq!(kept, [b"the new log".to_vec(), b"the old log".to_vec()]);
	drop(next);
	fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn stamps_sort_in_time_order() {
	let t = SystemTime::UNIX_EPOCH + std::time::Duration::from_millis(1_791_942_453_007);
	assert_eq!(stamp(t), "2026-10-14T01-47-33.007Z");
	let later = stamp(t + std::time::Duration::from_millis(993));
	assert!(later > stamp(t), "{later}");
}
