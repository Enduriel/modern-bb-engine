//! Script-path hashing (`ClassNameHash`) and small path helpers.
//!
//! Battle Brothers refers to every script by an extension-less, forward-slash
//! path (e.g. `scripts/skills/skill`). The engine stamps a stable integer hash of
//! that path — `ClassNameHash` — onto every object the module system builds, and
//! exposes a bidirectional path↔hash registry through `IO`.
//!
//! The original engine's exact hash algorithm is unconfirmed, and save-file
//! compatibility is out of scope, so this uses its own stable hash (FNV-1a). The
//! same function is used both to stamp objects and to answer `IO` queries, so it
//! round-trips with itself, which is all the module system needs.

/// Normalize a script path for use as a cache key and hash input: backslashes to
/// forward slashes, a trailing `.nut` removed, surrounding whitespace trimmed.
/// This folds the occasional stray variant onto the canonical form so a path is
/// hashed and cached consistently however it was written.
pub fn normalize_path(path: &str) -> String {
	let path = path.trim().replace('\\', "/");
	path.strip_suffix(".nut").unwrap_or(&path).to_string()
}

/// The final path segment — the BB "class name": `scripts/skills/actives/bash`
/// → `bash`. Used for the super-reference slot and for [`isKindOf`] matching.
///
/// [`isKindOf`]: crate::ModuleSystem
pub fn basename(path: &str) -> &str {
	let path = path.strip_suffix(".nut").unwrap_or(path);
	match path.rfind('/') {
		Some(slash) => &path[slash + 1..],
		None => path,
	}
}

/// FNV-1a (32-bit) hash of the normalized path, widened to the `i64` that
/// Squirrel integers use. Stable for a given path string; used both to stamp
/// `ClassNameHash` and to populate the `IO` registry.
pub fn class_name_hash(path: &str) -> i64 {
	const OFFSET_BASIS: u32 = 0x811c_9dc5;
	const PRIME: u32 = 0x0100_0193;

	let mut hash = OFFSET_BASIS;
	for byte in normalize_path(path).as_bytes() {
		hash ^= u32::from(*byte);
		hash = hash.wrapping_mul(PRIME);
	}
	// Reinterpret as a signed 32-bit value (the engine stores ClassNameHash via
	// writeI32), then widen — keeps the value in i32 range like the original.
	i64::from(hash as i32)
}

#[cfg(test)]
mod tests {
	use super::{basename, class_name_hash, normalize_path};

	#[test]
	fn normalizes_path_variants_to_one_form() {
		assert_eq!(
			normalize_path("scripts/skills/skill"),
			"scripts/skills/skill"
		);
		assert_eq!(
			normalize_path("scripts/skills/skill.nut"),
			"scripts/skills/skill"
		);
		assert_eq!(
			normalize_path("scripts\\skills\\skill"),
			"scripts/skills/skill"
		);
		assert_eq!(
			normalize_path("  scripts/skills/skill  "),
			"scripts/skills/skill"
		);
	}

	#[test]
	fn basename_is_the_last_segment() {
		assert_eq!(basename("scripts/skills/actives/bash"), "bash");
		assert_eq!(basename("scripts/skills/actives/bash.nut"), "bash");
		assert_eq!(basename("skill"), "skill");
	}

	#[test]
	fn hash_is_stable_and_path_invariant() {
		// The same logical path hashes the same however it is written.
		let h = class_name_hash("scripts/skills/skill");
		assert_eq!(h, class_name_hash("scripts/skills/skill.nut"));
		assert_eq!(h, class_name_hash("scripts\\skills\\skill"));
		// Distinct paths hash differently.
		assert_ne!(h, class_name_hash("scripts/skills/actives/bash"));
	}
}
