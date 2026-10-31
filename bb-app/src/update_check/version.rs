//! Release versions as the engine's tags write them (`v0.2.0`, or `0.2.0`): SemVer's
//! `major.minor.patch` with an optional pre-release (`-rc.1`), which sorts below its
//! release, and build metadata (`+…`), which is ignored. Only what SemVer allows is
//! read: no leading zeros, no missing parts, no stray text.

use std::cmp::Ordering;
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
	pub major: u64,
	pub minor: u64,
	pub patch: u64,
	/// The pre-release's dot-separated identifiers (empty for a release).
	pub pre: Vec<String>,
}

/// A number as SemVer writes it: digits, no leading zero (but `0` itself).
fn number(text: &str) -> Option<u64> {
	let digits = !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit());
	if !digits || (text.len() > 1 && text.starts_with('0')) {
		return None;
	}
	text.parse().ok()
}

/// Whether `id` is a pre-release identifier: letters, digits and `-`, a number without a
/// leading zero.
fn pre_identifier(id: &str) -> bool {
	if id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
		return false;
	}
	!id.bytes().all(|b| b.is_ascii_digit()) || number(id).is_some()
}

impl Version {
	/// `text` as a version, a leading `v` (or `V`) allowed; `None` if it isn't one.
	pub fn parse(text: &str) -> Option<Version> {
		let text = text.trim();
		let text = text.strip_prefix(['v', 'V']).unwrap_or(text);
		let text = text.split_once('+').map_or(text, |(version, build)| {
			// The build metadata, ignored, but it must be well formed.
			let ok = build.split('.').all(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'));
			if ok { version } else { "" }
		});
		let (core, pre) = match text.split_once('-') {
			Some((core, pre)) => (core, Some(pre)),
			None => (text, None),
		};
		let mut parts = core.split('.');
		let (major, minor, patch) = (number(parts.next()?)?, number(parts.next()?)?, number(parts.next()?)?);
		if parts.next().is_some() {
			return None;
		}
		let pre = match pre {
			None => Vec::new(),
			Some(pre) => {
				let ids: Vec<String> = pre.split('.').map(str::to_string).collect();
				if !ids.iter().all(|id| pre_identifier(id)) {
					return None;
				}
				ids
			}
		};
		Some(Version { major, minor, patch, pre })
	}
}

impl fmt::Display for Version {
	/// Without the `v`: `0.2.0`, `1.0.0-rc.1`.
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
		if !self.pre.is_empty() {
			write!(f, "-{}", self.pre.join("."))?;
		}
		Ok(())
	}
}

/// Two pre-release identifiers, as SemVer orders them: numbers by value, below text,
/// text in ASCII order.
fn compare_identifiers(a: &str, b: &str) -> Ordering {
	match (number(a), number(b)) {
		(Some(a), Some(b)) => a.cmp(&b),
		(Some(_), None) => Ordering::Less,
		(None, Some(_)) => Ordering::Greater,
		(None, None) => a.cmp(b),
	}
}

impl Ord for Version {
	fn cmp(&self, other: &Self) -> Ordering {
		(self.major, self.minor, self.patch)
			.cmp(&(other.major, other.minor, other.patch))
			.then_with(|| match (self.pre.is_empty(), other.pre.is_empty()) {
				(true, true) => Ordering::Equal,
				// A pre-release sorts below its release.
				(true, false) => Ordering::Greater,
				(false, true) => Ordering::Less,
				(false, false) => {
					for (a, b) in self.pre.iter().zip(&other.pre) {
						let order = compare_identifiers(a, b);
						if order != Ordering::Equal {
							return order;
						}
					}
					self.pre.len().cmp(&other.pre.len())
				}
			})
	}
}

impl PartialOrd for Version {
	fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
		Some(self.cmp(other))
	}
}

#[cfg(test)]
mod tests {
	use super::Version;

	fn v(text: &str) -> Version {
		Version::parse(text).unwrap_or_else(|| panic!("{text} should parse"))
	}

	#[test]
	fn tags_parse_with_or_without_the_v() {
		assert_eq!(v("v0.2.0"), Version { major: 0, minor: 2, patch: 0, pre: vec![] });
		assert_eq!(v("0.2.0"), v("v0.2.0"));
		assert_eq!(v("V1.10.3"), Version { major: 1, minor: 10, patch: 3, pre: vec![] });
		assert_eq!(v("1.0.0-rc.1").pre, ["rc", "1"]);
		assert_eq!(v("1.0.0+build.5"), v("1.0.0"), "build metadata is ignored");
		assert_eq!(v(" v2.0.0 ").to_string(), "2.0.0");
		assert_eq!(v("v1.0.0-alpha.2").to_string(), "1.0.0-alpha.2");
	}

	#[test]
	fn what_isnt_a_version_is_refused() {
		for text in [
			"", "v", "1", "1.2", "1.2.3.4", "01.2.3", "1.02.3", "1.2.03", "1.2.3-", "1.2.3-a..b", "1.2.3-01",
			"1.2.3-a_b", "1.2.3+", "1.2.3+a..b", "+1.2.3", "-1.2.3", "1.2.x", "vv1.2.3", "1.2.3 x", "latest",
			"1.2.99999999999999999999",
		] {
			assert_eq!(Version::parse(text), None, "{text:?}");
		}
	}

	#[test]
	fn versions_order_as_semver_says() {
		// SemVer's own example, in order.
		let ordered = [
			"1.0.0-alpha", "1.0.0-alpha.1", "1.0.0-alpha.beta", "1.0.0-beta", "1.0.0-beta.2", "1.0.0-beta.11",
			"1.0.0-rc.1", "1.0.0", "1.0.1", "1.1.0", "1.10.0", "2.0.0",
		];
		for pair in ordered.windows(2) {
			assert!(v(pair[0]) < v(pair[1]), "{} < {}", pair[0], pair[1]);
		}
		assert!(v("0.10.0") > v("0.9.9"), "by number, not text");
		assert_eq!(v("v0.2.0").cmp(&v("0.2.0")), std::cmp::Ordering::Equal);
	}
}
