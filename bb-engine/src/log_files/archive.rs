//! Keeping an earlier session's log: [`archive`] compresses it into a zip of its own
//! in `logs/`, written so that the log is never lost on the way.

use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

/// How old a temporary file left in `logs/` must be before a start removes it (an
/// archive being written by another instance right now is younger).
const STALE_TEMP: Duration = Duration::from_secs(600);

/// Compress `file` into `logs/<kind>-<stamp>.zip` holding just it, under `entry`, then
/// remove it; the zip's path. The zip is written under a temporary name, synced, read
/// back and checked (one entry, the same bytes by its CRC), then linked to its name
/// without replacing any zip there (a later one gets `-2`, `-3`, …); only then does
/// `file` go. Any failure leaves `file` where it was.
pub fn archive(file: &Path, logs: &Path, kind: &str, stamp: &str, entry: &str) -> io::Result<PathBuf> {
	fs::create_dir_all(logs)?;
	let temp = logs.join(format!(".{kind}-{stamp}.{}.tmp", std::process::id()));
	let result = write_zip(file, &temp, entry).and_then(|len| {
		check_zip(&temp, entry, len)?;
		place(&temp, logs, kind, stamp)
	});
	let _ = fs::remove_file(&temp);
	let zip = result?;
	sync_dir(logs);
	fs::remove_file(file)?;
	Ok(zip)
}

/// Write `file` into a new zip at `temp`, synced; the bytes it held.
fn write_zip(file: &Path, temp: &Path, entry: &str) -> io::Result<u64> {
	let mut source = File::open(file)?;
	let len = source.metadata()?.len();
	let mut options = SimpleFileOptions::default()
		.compression_method(CompressionMethod::Deflated)
		.large_file(len >= u64::from(u32::MAX));
	if let Some(time) = source.metadata()?.modified().ok().and_then(zip_time) {
		options = options.last_modified_time(time);
	}
	let mut zip = ZipWriter::new(File::create_new(temp)?);
	zip.start_file(entry, options)?;
	let copied = io::copy(&mut source, &mut zip)?;
	let out = zip.finish()?;
	out.sync_all()?;
	if copied != len {
		return Err(io::Error::other("the log changed while it was archived"));
	}
	Ok(copied)
}

/// Read the zip at `temp` back: exactly one entry, `entry`, of `len` bytes, whose CRC
/// matches (the reader checks it at the end).
fn check_zip(temp: &Path, entry: &str, len: u64) -> io::Result<()> {
	let mut zip = ZipArchive::new(File::open(temp)?)?;
	if zip.len() != 1 {
		return Err(io::Error::other("the archive has more than the log"));
	}
	let mut file = zip.by_index(0)?;
	if file.name() != entry {
		return Err(io::Error::other("the archive's entry is misnamed"));
	}
	let read = io::copy(&mut file, &mut io::sink())?;
	if read != len {
		return Err(io::Error::other("the archive's entry is short"));
	}
	Ok(())
}

/// Give the finished zip its name, never replacing another: a hard link fails on an
/// existing name (where links aren't supported, a rename after a check).
fn place(temp: &Path, logs: &Path, kind: &str, stamp: &str) -> io::Result<PathBuf> {
	for n in 1..1000 {
		let name = if n == 1 { format!("{kind}-{stamp}.zip") } else { format!("{kind}-{stamp}-{n}.zip") };
		let path = logs.join(name);
		match fs::hard_link(temp, &path) {
			Ok(()) => return Ok(path),
			Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
			Err(_) if !path.exists() => {
				fs::rename(temp, &path)?;
				return Ok(path);
			}
			Err(_) => continue,
		}
	}
	Err(io::Error::other("no free name for the archive"))
}

/// Make the directory's entries durable (Unix; elsewhere a no-op).
fn sync_dir(dir: &Path) {
	#[cfg(unix)]
	if let Ok(dir) = File::open(dir) {
		let _ = dir.sync_all();
	}
	#[cfg(not(unix))]
	let _ = dir;
}

/// Remove temporary files an interrupted archiving left in `logs/` (the log it was
/// archiving is still in place, so it's archived again).
pub fn remove_stale_temps(logs: &Path) {
	let Ok(entries) = fs::read_dir(logs) else { return };
	for entry in entries.flatten() {
		let name = entry.file_name();
		let name = name.to_string_lossy();
		let old = entry
			.metadata()
			.and_then(|m| m.modified())
			.is_ok_and(|t| t.elapsed().unwrap_or_default() > STALE_TEMP);
		if name.starts_with('.') && name.ends_with(".tmp") && old {
			let _ = fs::remove_file(entry.path());
		}
	}
}

/// A file time as a zip's (local time; zips have no zone), if it fits a zip's range.
fn zip_time(time: SystemTime) -> Option<zip::DateTime> {
	let ts = jiff::Timestamp::try_from(time).ok()?;
	let local = ts.to_zoned(jiff::tz::TimeZone::system());
	let narrow = |v: i8| u8::try_from(v).ok();
	zip::DateTime::from_date_and_time(
		u16::try_from(local.year()).ok()?,
		narrow(local.month())?,
		narrow(local.day())?,
		narrow(local.hour())?,
		narrow(local.minute())?,
		narrow(local.second())?,
	)
	.ok()
}

/// Read a whole small file (a slot's lock), from its start.
pub fn read_all(file: &mut File) -> io::Result<String> {
	file.seek(SeekFrom::Start(0))?;
	let mut text = String::new();
	file.read_to_string(&mut text)?;
	Ok(text)
}
