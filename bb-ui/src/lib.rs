//! The UI host: runs the game's HTML/JS UI (and mods' UI) in place of Coherent GT.
//!
//! The browser-independent pieces (always built):
//!   * [`json`] — the wire format, and [`sq_json`] — Squirrel values ↔ JSON;
//!   * [`bridge`] — the Coherent protocol on the game thread ([`bridge::UiProtocol`]);
//!   * [`coui`] — `coui://` URLs → files in the game's archives;
//!   * [`parity`] — page snapshots compared with the original's (UI parity goldens);
//!   * [`png`] — screenshots of the rendered UI.
//!
//! With the `cef` feature, [`host`] runs the page in Chromium (CEF), offscreen.

pub mod bridge;
pub mod coui;
pub mod json;
pub mod parity;
pub mod png;
pub mod sq_json;

#[cfg(feature = "cef")]
pub mod host;
