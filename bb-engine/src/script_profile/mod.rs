//! Reports of where the scripts' time goes ([`squirrel::ScriptProfiler`]), by function,
//! file and owner (the game's data or the mod each script came from): the
//! `script_profile` example's and `bb`'s (`BB_SCRIPT_PROFILE`).

mod owners;
mod report;

pub use owners::Owners;
pub use report::report;
