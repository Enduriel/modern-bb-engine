//! The engine's update notice on the page (see the file): shown and taken down by the
//! game, answering through the engine's own `bb.updateNotice` handler
//! ([`crate::bridge::ENGINE_PREFIX`]).

use super::UiSession;
use crate::json::Json;

/// Shows, hides or asks after the notice (see the file).
const NOTICE: &str = include_str!("update_notice.js");

/// The engine's handler the notice's buttons call: `(choice[, version])`, choice
/// `open`, `continue` or `ignore`.
pub const UPDATE_NOTICE_HANDLER: &str = "bb.updateNotice";

/// What the update notice says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateNoticeView {
	/// The newer version found.
	pub version: String,
	/// The version running.
	pub current: String,
	/// The release's notes, Markdown, shown as text.
	pub notes: String,
	/// What "Open link" opens (through the page's `openURL`).
	pub link: String,
}

impl UiSession {
	/// Show the update notice over the page, modal (replacing one showing).
	pub fn show_update_notice(&self, view: &UpdateNoticeView) -> Result<(), String> {
		let data = Json::Object(vec![
			("version".into(), Json::Str(view.version.clone())),
			("current".into(), Json::Str(view.current.clone())),
			("notes".into(), Json::Str(view.notes.clone())),
			("link".into(), Json::Str(view.link.clone())),
		]);
		self.eval(&format!("{NOTICE}('show', {})", data.to_text()))?;
		Ok(())
	}

	/// Take the update notice down, if it shows.
	pub fn hide_update_notice(&self) -> Result<(), String> {
		self.eval(&format!("{NOTICE}('hide')"))?;
		Ok(())
	}

	/// Choose `choice` (`open`, `continue` or `ignore`) as the notice's button does, if it
	/// shows: its handler and the page's `openURL` run before this returns. Whether it
	/// showed.
	pub fn choose_update_notice(&self, choice: &str) -> Result<bool, String> {
		Ok(self.eval(&format!("{NOTICE}('choose', {})", Json::Str(choice.into()).to_text()))? == Json::Bool(true))
	}

	/// Whether the update notice shows.
	pub fn update_notice_shown(&self) -> Result<bool, String> {
		Ok(self.eval(&format!("{NOTICE}('shown')"))? == Json::Bool(true))
	}
}
