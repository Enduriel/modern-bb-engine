//! UI parity: comparing our page with snapshots of the original's.
//!
//! [`SNAPSHOT_JS`] (`snapshot.js`) runs both in the original game (through a probe
//! runner that launches it, which captures the goldens) and in our host. It records the
//! visible element tree: tag, classes, own text, the box rounded to pixels, and a fixed
//! set of computed styles.
//!
//! [`compare`] aligns the two trees and lists what differs:
//! - siblings are matched by tag and classes, with a longest-common-subsequence
//!   alignment, so one missing or extra element doesn't misalign the rest;
//! - boxes may differ by up to [`Tolerance::pixels`];
//! - style values are normalized first ([`normalize_style`]), because the two engines
//!   spell the same value differently (`rgb()` vs `rgba()`, quoted font names).

mod tree;

pub use tree::{Difference, Node, Snapshot, Tolerance, compare, normalize_style};

/// The snapshot script (defines `BBSnapshot.take(root)`).
pub const SNAPSHOT_JS: &str = include_str!("snapshot.js");
