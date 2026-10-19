//! The UI host: runs the game's HTML/JS UI (and mods' UI) in place of Coherent GT.
//!
//! The browser-independent pieces (always built):
//!   * [`json`] — the wire format, and [`sq_json`] — Squirrel values ↔ JSON;
//!   * [`bridge`] — the Coherent protocol on the game thread ([`bridge::UiProtocol`]);
//!   * [`coui`] — `coui://` URLs → files in the game's archives;
//!   * [`hot_reload`] — which loose UI files changed, for restyling a running page;
//!   * [`mod_fonts`] — the `@font-face` rules mods' registered fonts add to the page;
//!   * [`parity`] — page snapshots compared with the original's (UI parity goldens);
//!   * [`png`] — screenshots of the rendered UI;
//!   * [`web_url`] — which URLs the page may have opened in the system's browser.
//!
//! With the `cef` feature, [`host`] runs the page in Chromium (CEF), offscreen.

pub mod bridge;
pub mod coui;
pub mod hot_reload;
pub mod json;
pub mod mod_fonts;
pub mod parity;
pub mod png;
pub mod sq_json;
pub mod web_url;

#[cfg(feature = "cef")]
pub mod host;
