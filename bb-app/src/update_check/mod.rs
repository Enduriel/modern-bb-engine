//! The update check: at start, bb asks GitHub for the engine's latest release and, if
//! it is newer than bb, the main menu shows a notice of it (the page's front, see
//! `front::update_notice`). Notify only: nothing is downloaded or installed.
//!
//! What is decided here, away from the browser:
//! - [`version`]: tags as versions, and which is newer;
//! - [`link`]: where the notice's "Open link" goes;
//! - [`release`] (with the page's front): the release's fields from GitHub's answer;
//! - [`decide`]: whether a release makes a notice, and what it says.
//!
//! The player can ignore a version: it's kept in the settings file
//! ([`IGNORED_VERSION`]) and that version never shows again, a later one does. There is
//! no way to turn the check off.

pub mod link;
#[cfg(feature = "ui")]
pub mod release;
pub mod version;

use version::Version;

/// GitHub's latest release of the engine: drafts and pre-releases are never "latest".
pub const ENDPOINT: &str = "https://api.github.com/repos/Enduriel/modern-bb-engine/releases/latest";

/// For developers and tests only: another address to ask in place of [`ENDPOINT`] (a
/// local server's).
pub const ENDPOINT_ENV: &str = "BB_UPDATE_CHECK_URL";

/// The settings file's key for the version the player chose to ignore.
pub const IGNORED_VERSION: &str = "update.ignored_version";

/// The most of the release notes the notice shows, in characters.
const MAX_NOTES: usize = 6000;

/// The address the check asks: [`ENDPOINT`], or [`ENDPOINT_ENV`]'s.
pub fn endpoint() -> String {
	std::env::var(ENDPOINT_ENV).ok().filter(|url| !url.is_empty()).unwrap_or_else(|| ENDPOINT.to_string())
}

/// What the update check is given (`Boot::update_check`): the game gives it
/// [`endpoint`] and its settings file; tests a local server.
#[derive(Clone, Debug)]
pub struct UpdateCheck {
	/// The address asked.
	pub endpoint: String,
	/// The settings file the ignored version is read from and kept in; without one it's
	/// neither.
	pub settings: Option<std::path::PathBuf>,
}

/// What a release says that the check uses.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Release {
	/// Its tag (`v0.3.0`).
	pub tag: String,
	/// Its page on GitHub.
	pub html_url: String,
	/// Its notes (Markdown).
	pub notes: String,
}

/// What the notice says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
	/// The release's version, without the `v` (what "Ignore this version" keeps).
	pub version: String,
	/// bb's version.
	pub current: String,
	pub notes: String,
	/// Where "Open link" goes.
	pub link: String,
}

/// The notice `release` makes for bb at version `current`, given the version the player
/// ignored (if any) and the engine's Nexus page (if it has one): `Ok(None)` if it is no
/// newer or ignored, an error if it can't be read or has no usable link.
pub fn decide(release: &Release, current: &str, ignored: Option<&str>, nexus: Option<&str>) -> Result<Option<Notice>, String> {
	let found = Version::parse(&release.tag).ok_or_else(|| format!("the release's tag {:?} is no version", release.tag))?;
	let running = Version::parse(current).ok_or_else(|| format!("bb's version {current:?} is no version"))?;
	if found <= running || ignored.and_then(Version::parse).is_some_and(|ignored| ignored == found) {
		return Ok(None);
	}
	let link = link::choose(nexus, &release.html_url)
		.ok_or_else(|| format!("the release's page {:?} is not one of the engine's releases", release.html_url))?;
	let notes = match release.notes.char_indices().nth(MAX_NOTES) {
		Some((cut, _)) => format!("{}\n…", &release.notes[..cut]),
		None => release.notes.clone(),
	};
	Ok(Some(Notice { version: found.to_string(), current: running.to_string(), notes, link }))
}

#[cfg(test)]
mod tests {
	use super::*;

	fn release(tag: &str) -> Release {
		Release {
			tag: tag.into(),
			html_url: format!("https://github.com/Enduriel/modern-bb-engine/releases/tag/{tag}"),
			notes: "## Changes\n- faster".into(),
		}
	}

	#[test]
	fn a_newer_release_makes_a_notice() {
		let notice = decide(&release("v0.3.0"), "0.2.0", None, None).unwrap().expect("a notice");
		assert_eq!(
			notice,
			Notice {
				version: "0.3.0".into(),
				current: "0.2.0".into(),
				notes: "## Changes\n- faster".into(),
				link: "https://github.com/Enduriel/modern-bb-engine/releases/tag/v0.3.0".into(),
			}
		);
		let nexus = "https://nexusmods.com/battlebrothers/mods/42";
		let notice = decide(&release("0.3.0"), "0.2.0", None, Some(nexus)).unwrap().expect("a notice");
		assert_eq!(notice.link, "https://www.nexusmods.com/battlebrothers/mods/42");
		assert!(decide(&release("v0.2.1-rc.1"), "0.2.0", None, None).unwrap().is_some());
		assert!(decide(&release("v0.3.0"), "0.3.0-rc.2", None, None).unwrap().is_some(), "the release of a pre-release");
	}

	#[test]
	fn the_same_or_older_makes_none() {
		for tag in ["v0.2.0", "0.2.0", "v0.1.9", "v0.2.0-rc.1"] {
			assert_eq!(decide(&release(tag), "0.2.0", None, None), Ok(None), "{tag}");
		}
	}

	#[test]
	fn an_ignored_version_makes_none_but_a_later_one_does() {
		assert_eq!(decide(&release("v0.3.0"), "0.2.0", Some("0.3.0"), None), Ok(None));
		assert_eq!(decide(&release("v0.3.0"), "0.2.0", Some("v0.3.0"), None), Ok(None));
		assert!(decide(&release("v0.3.1"), "0.2.0", Some("0.3.0"), None).unwrap().is_some());
		assert!(decide(&release("v0.3.0"), "0.2.0", Some("garbage"), None).unwrap().is_some(), "an unreadable one ignores nothing");
	}

	#[test]
	fn what_cant_be_used_is_an_error() {
		assert!(decide(&release("latest"), "0.2.0", None, None).is_err());
		assert!(decide(&release("v1.0.0"), "dev", None, None).is_err());
		let elsewhere = Release { html_url: "https://example.com/v1".into(), ..release("v1.0.0") };
		assert!(decide(&elsewhere, "0.2.0", None, None).is_err());
	}

	#[test]
	fn long_notes_are_cut() {
		let long = Release { notes: "é".repeat(MAX_NOTES + 10), ..release("v1.0.0") };
		let notice = decide(&long, "0.2.0", None, None).unwrap().unwrap();
		assert_eq!(notice.notes.chars().count(), MAX_NOTES + 2);
		assert!(notice.notes.ends_with("\n…"));
	}

	#[test]
	fn bbs_own_version_reads() {
		assert!(Version::parse(env!("CARGO_PKG_VERSION")).is_some());
	}
}
