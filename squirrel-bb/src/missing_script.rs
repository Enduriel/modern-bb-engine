//! What `new` and `inherit` do with a script that doesn't exist, as the original's
//! (probed 2026-10-14): they log the failure as errors and give `null` instead of
//! throwing, so the script goes on (and fails later, where it uses the `null`).
//!
//! The original's log names the file it looked for by the path cut at its first `.`
//! (`scripts/skills/backgrounds/background.nope` looks for
//! `scripts/skills/backgrounds/background.(nut/cnut)`); `inherit` logs that twice.

use squirrel::{SQVM, SqStrongRef};

use crate::error::ScriptError;
use crate::html_log::{self, Class};

/// The original's line for a script file it can't find.
fn file_not_found(path: &str) -> String {
	let stem = path.split('.').next().unwrap_or(path);
	format!("Failed to load script file \"{stem}.(nut/cnut)\". Reason: File not found.")
}

/// Log `lines` as errors of the script system (the original's tag `Script`).
fn log_errors(_vm: &SQVM, lines: &[String]) {
	for line in lines {
		html_log::engine(Class::Error, "Script", line);
	}
}

/// `new(path)`'s result: a missing script is logged and gives `None` (null).
pub(crate) fn new_or_null(vm: &SQVM, path: &str, result: Result<SqStrongRef, ScriptError>) -> Result<Option<SqStrongRef>, ScriptError> {
	match result {
		Ok(instance) => Ok(Some(instance)),
		Err(ScriptError::NotFound(missing)) => {
			log_errors(vm, &[
				file_not_found(&missing),
				format!("Failed to create new instance from \"{path}\". Reason: Script not found."),
			]);
			Ok(None)
		}
		Err(e) => Err(e),
	}
}

/// `inherit(path, body)`'s result: a missing parent is logged and gives `None` (null).
pub(crate) fn inherit_or_null(vm: &SQVM, path: &str, result: Result<SqStrongRef, ScriptError>) -> Result<Option<SqStrongRef>, ScriptError> {
	match result {
		Ok(class) => Ok(Some(class)),
		Err(ScriptError::NotFound(missing)) => {
			let looked = file_not_found(&missing);
			log_errors(vm, &[looked.clone(), looked, format!("Failed to inherit from \"{path}\". Reason: Script not found.")]);
			Ok(None)
		}
		Err(e) => Err(e),
	}
}

#[cfg(test)]
mod tests {
	use super::file_not_found;

	#[test]
	fn the_file_looked_for_is_the_path_up_to_its_first_dot() {
		assert_eq!(
			file_not_found("scripts/skills/backgrounds/background.nope"),
			"Failed to load script file \"scripts/skills/backgrounds/background.(nut/cnut)\". Reason: File not found."
		);
		assert_eq!(file_not_found("scripts/nope/missing_thing"), "Failed to load script file \"scripts/nope/missing_thing.(nut/cnut)\". Reason: File not found.");
	}
}
