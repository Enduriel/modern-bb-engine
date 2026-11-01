//! Files that a mod provides and another mod (or a loose file) replaces: likely mod
//! conflicts, reported in the log at startup.
//!
//! A mod replacing one of the game's own files is how mods work, so that is never
//! reported. What is: a file several mods provide (only one copy is used, the others'
//! silently not), and a mod's file the game's own copy beats (a `.nut` no newer than
//! vanilla's `.cnut`, or a mod archive that loads before vanilla's). Copies with the same
//! content don't conflict.
//!
//! Which copy wins is the engine's own rule ([`DatScriptSource`]): the data folder's loose
//! files, then the archives from the last loaded; for a script, its `.nut` or `.cnut`,
//! whichever is newer ([`source_is_newer`]).

mod provider;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};

use crate::datsource::{DatScriptSource, source_is_newer};
use provider::Provider;

/// How many of a conflict's files its message names.
const NAMED: usize = 10;

/// Files of `loser` that aren't used because `winner` has them too.
#[derive(Debug, PartialEq)]
pub struct Conflict {
	pub winner: String,
	/// Whether `winner` is one of the game's own archives (`data_*.dat`).
	pub winner_vanilla: bool,
	pub loser: String,
	/// The paths of `loser`'s unused files, each with why the winner's copy is used when
	/// that isn't the search order alone (a script's other extension).
	pub files: Vec<(String, Option<&'static str>)>,
}

impl Conflict {
	/// The warning for the log.
	pub fn message(&self) -> String {
		let count = self.files.len();
		let plural = if count == 1 { "" } else { "s" };
		let mut named: Vec<String> = self
			.files
			.iter()
			.take(NAMED)
			.map(|(path, why)| match why {
				Some(why) => format!("{path} ({why})"),
				None => path.clone(),
			})
			.collect();
		if count > NAMED {
			named.push(format!("and {} more", count - NAMED));
		}
		let named = named.join(", ");
		let own = if self.winner_vanilla { " (the game's own)" } else { "" };
		format!("\"{}\"{own} overrides {count} file{plural} of \"{}\": {named}", self.winner, self.loser)
	}
}

/// Every conflict among the files `source` mounts, in the order of the paths.
pub fn find(source: &DatScriptSource) -> Vec<Conflict> {
	source.with_archives(|archives| {
		let providers = provider::in_search_order(archives, source.mounts());
		conflicts(&providers)
	})
}

/// [`find`], each conflict logged as a warning (to log.html, when one is open, and stderr).
pub fn log(source: &DatScriptSource) {
	for conflict in find(source) {
		let message = conflict.message();
		squirrel_bb::html_log::line(squirrel_bb::html_log::Class::Warning, "Mods", &message);
		eprintln!("[Warning] {message}");
	}
}

/// A script path without its extension, or `None` for a file that isn't a script.
fn script_stem(path: &str) -> Option<&str> {
	path.strip_suffix(".nut").or_else(|| path.strip_suffix(".cnut"))
}

/// A script's extension (`.nut` or `.cnut`), or `None` for a file that isn't a script.
fn script_extension(path: &str) -> Option<&'static str> {
	[".nut", ".cnut"].into_iter().find(|ext| path.ends_with(ext))
}

/// The conflicts among `providers` (in search order: the first that has a file wins).
fn conflicts(providers: &[Provider]) -> Vec<Conflict> {
	// Only a path some mod or loose folder has can conflict: bucket theirs by path (a
	// script under either extension is one), then look each up in vanilla's archives,
	// which hold most of the files and need no listing.
	let mut by_key: BTreeMap<(String, bool), Vec<(usize, String)>> = BTreeMap::new();
	for (i, provider) in providers.iter().enumerate().filter(|(_, p)| !p.vanilla) {
		for name in provider.names() {
			let key = script_stem(&name).map_or((name.clone(), false), |stem| (stem.to_string(), true));
			by_key.entry(key).or_default().push((i, name));
		}
	}
	let mut out: Vec<Conflict> = Vec::new();
	for ((key, script), mut copies) in by_key {
		let names = if script { vec![format!("{key}.nut"), format!("{key}.cnut")] } else { vec![key] };
		for (i, provider) in providers.iter().enumerate().filter(|(_, p)| p.vanilla) {
			copies.extend(names.iter().filter(|n| provider.has(n)).map(|n| (i, n.clone())));
		}
		if copies.iter().map(|c| c.0).collect::<BTreeSet<_>>().len() < 2 {
			continue;
		}
		// In search order.
		copies.sort();
		let copies: Vec<(usize, &str)> = copies.iter().map(|(i, n)| (*i, n.as_str())).collect();
		let (winner, won) = winning_copy(providers, &copies);
		for &(i, name) in &copies {
			let loser = &providers[i];
			if i == winner || loser.vanilla || providers[winner].same_content(won, loser, name) {
				continue;
			}
			let why = match (script_extension(won), script_extension(name)) {
				(Some(".cnut"), Some(".nut")) => Some("its .cnut is as new or newer"),
				(Some(".nut"), Some(".cnut")) => Some("its .nut is newer"),
				_ => None,
			};
			let winner = &providers[winner];
			match out.iter_mut().find(|c| c.winner == winner.label && c.loser == loser.label) {
				Some(conflict) => conflict.files.push((name.to_string(), why)),
				None => out.push(Conflict {
					winner: winner.label.clone(),
					winner_vanilla: winner.vanilla,
					loser: loser.label.clone(),
					files: vec![(name.to_string(), why)],
				}),
			}
		}
	}
	out
}

/// The copy the game uses among `copies` (in search order): the first, or for a script
/// the first `.nut` or the first `.cnut`, whichever is newer.
fn winning_copy<'a>(providers: &[Provider], copies: &[(usize, &'a str)]) -> (usize, &'a str) {
	let first = |ext| copies.iter().find(|c| script_extension(c.1) == Some(ext)).copied();
	match (first(".nut"), first(".cnut")) {
		(Some(nut), Some(cnut)) => {
			let modified = |(i, name): (usize, &str)| providers[i].modified(name);
			if source_is_newer(modified(nut), modified(cnut)) { nut } else { cnut }
		}
		_ => copies[0],
	}
}
