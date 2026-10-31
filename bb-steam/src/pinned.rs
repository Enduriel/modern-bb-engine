//! The one copy of Steam's redistributable library a release package carries: the
//! 64-bit libraries vendored in the `steamworks-sys` crate on crates.io, pinned to one
//! version, with the SHA-256 of the published `.crate` (crates.io's own checksum) and of
//! each library inside it hard-coded here. The packaging fetches the crate, checks both,
//! and refuses anything else, so a changed upstream can't be swapped in.
//!
//! (Battle Brothers' own `win32/steam_api.dll` is 32-bit, so a 64-bit bb can't load it.)
//! Updating the pin is a deliberate change: new version, new hashes, reviewed.

/// The pinned `steamworks-sys` version (published 2026-04-14; Steamworks SDK with
/// `SteamAPI_InitFlat` and `STEAMUSERSTATS_INTERFACE_VERSION013`).
pub const CRATE_VERSION: &str = "0.13.0";

/// SHA-256 of `steamworks-sys-0.13.0.crate`, as crates.io publishes it.
pub const CRATE_SHA256: &str = "ae139f051204a4af015d4f0ed3a1f7a51020d03a44c7cf249168c543d88131e9";

/// Where crates.io serves the pinned crate.
pub fn crate_url() -> String {
	format!("https://static.crates.io/crates/steamworks-sys/steamworks-sys-{CRATE_VERSION}.crate")
}

/// The file name of the pinned crate.
pub fn crate_file() -> String {
	format!("steamworks-sys-{CRATE_VERSION}.crate")
}

/// One target's library in the pinned crate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PinnedLibrary {
	/// The Rust target triples it serves.
	pub targets: &'static [&'static str],
	/// Its path inside the crate's archive.
	pub member: &'static str,
	/// The name bb looks for beside its executable.
	pub file_name: &'static str,
	/// Its SHA-256.
	pub sha256: &'static str,
}

/// Every pinned library, by target.
pub const LIBRARIES: [PinnedLibrary; 4] = [
	PinnedLibrary {
		targets: &["x86_64-unknown-linux-gnu"],
		member: "steamworks-sys-0.13.0/lib/steam/redistributable_bin/linux64/libsteam_api.so",
		file_name: "libsteam_api.so",
		sha256: "ec4797f76a206eb0af627af0f0788eb5a2eabf5dee38ee0a6affbaa44a645f4e",
	},
	PinnedLibrary {
		targets: &["aarch64-unknown-linux-gnu"],
		member: "steamworks-sys-0.13.0/lib/steam/redistributable_bin/linuxarm64/libsteam_api.so",
		file_name: "libsteam_api.so",
		sha256: "179bfb241de94c0320755b558d91edb0004f72d98bf7ad6f1147f9ada835dcbc",
	},
	PinnedLibrary {
		targets: &["x86_64-pc-windows-msvc", "x86_64-pc-windows-gnu", "x86_64-pc-windows-gnullvm"],
		member: "steamworks-sys-0.13.0/lib/steam/redistributable_bin/win64/steam_api64.dll",
		file_name: "steam_api64.dll",
		sha256: "eb17909a76668cf9ae0b92a618a34a50f6c73d3a6787cb4dd8ce36a8b10bfb75",
	},
	// A universal binary (x86_64 and arm64).
	PinnedLibrary {
		targets: &["x86_64-apple-darwin", "aarch64-apple-darwin"],
		member: "steamworks-sys-0.13.0/lib/steam/redistributable_bin/osx/libsteam_api.dylib",
		file_name: "libsteam_api.dylib",
		sha256: "1e2282e1032851e423b501c2c483586793f1d5e11585edaae4c78d6a3abad1eb",
	},
];

/// The pinned library for the Rust target `triple`.
pub fn library_for(triple: &str) -> Option<&'static PinnedLibrary> {
	LIBRARIES.iter().find(|l| l.targets.contains(&triple))
}

/// A target whose packages carry no Steam library, and why. bb built for it runs with
/// achievements and stats off, as it does wherever the library is missing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NoLibrary {
	pub target: &'static str,
	pub why: &'static str,
}

/// The targets packaged without Steam's library: a deliberate entry each, so a missing
/// library is never a silent gap.
pub const WITHOUT_LIBRARY: [NoLibrary; 1] = [NoLibrary {
	target: "aarch64-pc-windows-msvc",
	why: "the pinned steamworks-sys has no Windows ARM64 steam_api (only win64's x86_64 \
	      steam_api64.dll), and an ARM64 process can't load an x86_64 DLL",
}];

/// Why `triple`'s packages carry no Steam library, if they don't.
pub fn without_library(triple: &str) -> Option<&'static NoLibrary> {
	WITHOUT_LIBRARY.iter().find(|n| n.target == triple)
}

/// Check that `actual` (lowercase hex SHA-256 of `what`) is `expected`.
pub fn verify(what: &str, actual: &str, expected: &str) -> Result<(), String> {
	if actual.eq_ignore_ascii_case(expected) {
		Ok(())
	} else {
		Err(format!("{what}: SHA-256 {actual}, but the pin is {expected}; refusing it"))
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn every_desktop_target_has_a_library_named_as_bb_looks_for_it() {
		for triple in ["x86_64-unknown-linux-gnu", "x86_64-pc-windows-msvc", "aarch64-apple-darwin", "x86_64-apple-darwin"] {
			assert!(library_for(triple).is_some(), "{triple}");
		}
		assert!(library_for("i686-pc-windows-msvc").is_none(), "32-bit is not shipped");
		let this = env!("BB_STEAM_TARGET");
		match library_for(this) {
			Some(library) => assert_eq!(library.file_name, crate::LIBRARY_NAME),
			None => assert!(without_library(this).is_some(), "{this} has neither a pinned Steam library nor a reason why not"),
		}
	}

	/// Every target a release bundle is made for has either a pinned library or a reason
	/// for none, never both.
	#[test]
	fn every_bundled_target_has_a_library_or_a_reason() {
		for triple in ["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu", "x86_64-pc-windows-msvc", "aarch64-pc-windows-msvc", "aarch64-apple-darwin"] {
			assert!(library_for(triple).is_some() != without_library(triple).is_some(), "{triple}");
		}
		assert_eq!(without_library("aarch64-pc-windows-msvc").map(|n| n.target), Some("aarch64-pc-windows-msvc"));
	}

	#[test]
	fn hashes_are_lowercase_sha256_and_members_in_the_pinned_crate() {
		let is_hash = |h: &str| h.len() == 64 && h.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
		assert!(is_hash(CRATE_SHA256));
		for library in LIBRARIES {
			assert!(is_hash(library.sha256), "{library:?}");
			assert!(library.member.starts_with(&format!("steamworks-sys-{CRATE_VERSION}/")), "{library:?}");
			assert!(library.member.ends_with(library.file_name), "{library:?}");
		}
	}

	/// The check accepts only the pinned hash.
	#[test]
	fn verification_refuses_anything_but_the_pin() {
		let data = b"not the steam library";
		let actual = bb_sha256::hex(data);
		assert!(verify("lib", &actual, &actual).is_ok());
		let err = verify("lib", &actual, LIBRARIES[0].sha256).unwrap_err();
		assert!(err.contains("refusing"), "{err}");
	}
}
