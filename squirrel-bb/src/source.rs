//! The seam between the module system and wherever script bytes come from.

/// What a [`ScriptSource`] hands back for a path: either Squirrel source text to
/// compile, or a closure already serialized in *this VM's* bytecode format (e.g.
/// a transcoded game `.cnut`) to load directly.
pub enum LoadedScript {
	/// Squirrel source text — compiled and run.
	Source(Vec<u8>),
	/// Bytecode in this VM's `sq_writeclosure` format — loaded via
	/// `SQVM::load_closure` and run. (The source is responsible for decrypting +
	/// transcoding a game `.cnut` to this VM's format before returning it.)
	Bytecode(Vec<u8>),
}

/// Resolves a script path (no extension, e.g. `scripts/entity/tactical/actor`) to
/// its contents.
///
/// This is where the VFS / file-replacement layer plugs in: the module system
/// calls [`load`](ScriptSource::load) whenever it must bring a script in, and the
/// higher-level host decides which mod or vanilla file actually answers (per the
/// BB "replacing files" rules — notably, a mod's `.nut` source overrides a vanilla
/// `.cnut`). Keeping it a trait keeps the VFS out of this crate.
///
/// Paths are passed already normalized (forward slashes, no extension), so an
/// implementation can key its storage on the path verbatim.
pub trait ScriptSource {
	/// Return the script at `path` (as source or bytecode), or `None` if no file
	/// provides it.
	fn load(&self, path: &str) -> Option<LoadedScript>;

	/// List every script under `prefix`, **recursively**, as `load`-able
	/// extension-less paths in deterministic (lexicographic) order — or `None` if
	/// `prefix` names no directory.
	///
	/// This backs `IO.enumerateFiles`, which `preload.nut` calls to sweep
	/// `scripts/config/`, `scripts/dlc/`, and `scripts/`. The `None` return is
	/// meaningful: the DLC sweep is guarded against it, so a missing `scripts/dlc/`
	/// must return `None` (not an empty array). `prefix` may or may not end in `/`;
	/// it is matched at a path-segment boundary either way.
	///
	/// Implementations must:
	/// - return paths in the same extension-less, forward-slash form `load` takes
	///   (strip any `.nut`), recursing into subdirectories;
	/// - **never panic** — this is reached through the native trampoline (an
	///   `extern "C"` boundary where unwinding is UB), so a filesystem error must
	///   become `None`/empty, not an `unwrap`.
	fn list_prefix(&self, prefix: &str) -> Option<Vec<String>>;
}
