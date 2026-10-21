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
mod collect;
pub use collect::Discard;
mod compile_context;
mod engine;
mod font_file;
pub mod frame_timing;
pub mod modern_engine;
mod error;
mod error_report;
pub use error_report::{report_script_errors, terminal_text};
mod hash;
pub mod html_log;
mod input_events;
mod io_table;
mod key_queue;
mod missing_script;
mod module;
mod native;
pub mod platform;
pub mod scene;
mod source;
mod state;
mod state_stack;
pub mod unicode_ranges;
pub mod ui_client;

pub use engine::Engine;
pub use error::ScriptError;
pub use hash::{basename, class_name_hash, normalize_path};
pub use module::{FileReader, Instantiator, ModuleSystem};
pub use native::{
	BattleCamera, BattleMotion, Color, DLC_SCRIPTS, ExactClock, ExtensionSection, LogLevel, NativeConfig, SaveFormat, SaveSystem, compare_original_saves, original_save_name, TimeService, UiBridge, UiCall, UiFrontend, Vec2f, Vec2i,
	WorldGrid, install_logging, install_natives, installed_dlc,
};
pub use source::{LoadedScript, ScriptSource};
pub use input_events::{KEY_MODIFIER_COMMAND, KeyInput, MouseInput};
pub use key_queue::QueuedInput;
pub use state_stack::RootState;
pub use native::user_stats::{OfflineStats, UserStats, UserStatsHandle};
pub use native::{EngineRng, FOG_MASK_SIZE, FogPatch, FogStamp, GameSettings, SavedSettings, Sprites, VideoMode, VideoSettings};
pub use native::{GAMEPLAY as GAMEPLAY_SETTINGS, VOLUMES as SOUND_VOLUMES};
pub use native::ui_cursor::CursorLook;
pub use native::audio::{Audio, AudioOut, NullOut, Playing, Voice, sound_length_ms, sound_length_ms_in};
