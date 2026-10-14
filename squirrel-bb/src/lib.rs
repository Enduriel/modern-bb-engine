//! Battle Brothers-specific Squirrel conventions, built on the `squirrel` crate.
//!
//! This crate adds the BB **module system** — the native `this.inherit` /
//! `this.new` / `this.include` functions, the prototype cache, `ClassNameHash`
//! stamping, member-table deep-merge, `isKindOf`, and the `IO` hash registry —
//! that all `.nut` game scripts rely on.
//!
//! It deliberately does **not** contain:
//! - generic Squirrel VM bindings (those live in `squirrel`), nor
//! - the **VFS** (path → bytes resolution / file-replacement). The VFS is managed
//!   at a higher level and injected via [`ScriptSource`]; this crate only asks
//!   "give me the source for this script path".
//!
//! Above this crate sits the higher-level engine crate.

mod class;
mod compile_context;
mod engine;
mod error;
mod hash;
mod input_events;
mod io_table;
mod key_queue;
mod module;
mod native;
pub mod scene;
mod source;
mod state;
mod state_stack;
pub mod ui_client;

pub use engine::Engine;
pub use error::ScriptError;
pub use hash::{basename, class_name_hash, normalize_path};
pub use module::{FileReader, Instantiator, ModuleSystem};
pub use native::{
	BattleCamera, Color, DLC_SCRIPTS, ExactClock, LogLevel, NativeConfig, SaveFormat, SaveSystem, compare_original_saves, TimeService, UiBridge, UiCall, UiFrontend, Vec2f, Vec2i,
	WorldGrid, install_logging, install_natives, installed_dlc,
};
pub use source::{LoadedScript, ScriptSource};
pub use input_events::{KeyInput, MouseInput};
pub use state_stack::RootState;
pub use native::{EngineRng, FOG_MASK_SIZE, FogPatch, FogStamp, Sprites, VideoSettings};
pub use native::audio::{Audio, AudioOut, NullOut, Playing, Voice, sound_length_ms};
