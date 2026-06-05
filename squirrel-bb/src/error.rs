//! Errors surfaced while loading, compiling, or running scripts.

/// Something that went wrong bringing a script in through the module system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptError {
	/// The [`ScriptSource`](crate::ScriptSource) had no bytes for this path.
	NotFound(String),
	/// The script failed to compile.
	Compile(String),
	/// The script raised an error while running (or a constructor threw).
	Runtime(String),
}

impl std::fmt::Display for ScriptError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			ScriptError::NotFound(p) => write!(f, "script not found: {p}"),
			ScriptError::Compile(e) => write!(f, "compile error: {e}"),
			ScriptError::Runtime(e) => write!(f, "runtime error: {e}"),
		}
	}
}

impl std::error::Error for ScriptError {}
