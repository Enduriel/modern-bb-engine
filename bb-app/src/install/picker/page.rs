//! The not-found page: its HTML (`page.html`, with the places looked in filled in), and
//! what it asks of bb (`engine.SendMessage('bbInstall', 0, action, value)`).

use bb_engine::install::Looked;
use bb_ui::json::Json;

use super::folder_dialog::Answer;

/// What the page asks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
	/// Open the folder dialog, at the folder typed so far if any.
	Browse(String),
	/// Use the folder typed or chosen.
	Use(String),
	/// Quit without a folder.
	Quit,
}

/// The page, as a `data:` URL (it loads nothing else), with how to name the folder in
/// the settings file at `settings_file` if there is one.
pub fn url(looked: &[Looked], settings_file: Option<&std::path::Path>) -> String {
	// `<` escaped: a folder's name can't end the script (`</script>`).
	let script_text = |json: Json| json.to_text().replace('<', "\\u003c");
	let looked = script_text(Json::Array(looked.iter().map(|l| Json::Str(l.to_string())).collect()));
	let example = script_text(Json::Str(example().to_string()));
	let settings_file = script_text(Json::Str(settings_file.map(|f| f.display().to_string()).unwrap_or_default()));
	let key = script_text(Json::Str(crate::settings::CHOSEN_DATA_DIR.to_string()));
	let example_data = script_text(Json::Str(crate::install::example_data_dir().to_string()));
	let html = include_str!("page.html")
		.replacen("/*LOOKED*/[]", &looked, 1)
		.replacen("/*EXAMPLE*/''", &example, 1)
		.replacen("/*SETTINGS_FILE*/''", &settings_file, 1)
		.replacen("/*KEY*/''", &key, 1)
		.replacen("/*EXAMPLE_DATA*/''", &example_data, 1);
	format!("data:text/html;charset=utf-8,{}", percent_encode(&html))
}

/// Where Steam installs the game on this system, as the page's example.
fn example() -> &'static str {
	if cfg!(windows) {
		r"C:\Program Files (x86)\Steam\steamapps\common\Battle Brothers"
	} else if cfg!(target_os = "macos") {
		"~/Library/Application Support/Steam/steamapps/common/Battle Brothers"
	} else {
		"~/.local/share/Steam/steamapps/common/Battle Brothers"
	}
}

/// `text` with every byte but the URL-safe ones percent-escaped.
fn percent_encode(text: &str) -> String {
	let mut out = String::with_capacity(text.len() * 3 / 2);
	for byte in text.bytes() {
		if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
			out.push(byte as char);
		} else {
			out.push_str(&format!("%{byte:02X}"));
		}
	}
	out
}

/// The page's call `message` (`[name, id, action, value]`) as an action, if it's one.
pub fn action(message: &Json) -> Option<Action> {
	let [name, _id, action, rest @ ..] = message.as_array()? else { return None };
	if name.as_str()? != "bbInstall" {
		return None;
	}
	let value = rest.first().and_then(Json::as_str).unwrap_or("").to_string();
	match action.as_str()? {
		"browse" => Some(Action::Browse(value)),
		"use" => Some(Action::Use(value)),
		"quit" => Some(Action::Quit),
		_ => None,
	}
}

/// The JS that hands the page the folder dialog's answer.
pub fn answer_script(answer: &Answer) -> String {
	match answer {
		Answer::Folder(folder) => format!("bbInstall.picked({})", Json::Str(folder.to_string_lossy().into_owned()).to_text()),
		Answer::Cancelled => "bbInstall.cancelled(false)".to_string(),
		Answer::Unavailable => "bbInstall.cancelled(true)".to_string(),
	}
}

/// The JS that tells the page why the folder isn't the game's.
pub fn refused_script(why: &str) -> String {
	format!("bbInstall.refused({})", Json::Str(why.to_string()).to_text())
}

#[cfg(test)]
mod tests {
	use super::*;
	use bb_engine::install::Source;

	#[test]
	fn the_page_lists_where_bb_looked() {
		let looked = [Looked::new(Source::Remembered, "/games/<BB> & co", "gone")];
		let url = url(&looked, Some(std::path::Path::new("/home/me/.config/battle-brothers-modern/settings.cfg")));
		assert!(url.starts_with("data:text/html;charset=utf-8,%3C%21doctype"), "{}", &url[..60]);
		assert!(!url.contains(' ') && !url.contains('<') && !url.contains('#'), "escaped");
		assert!(url.contains(&percent_encode(r#"var looked = ["/games/\u003cBB> & co: gone"]"#)), "the list, with `<` escaped");
		assert!(url.contains(&percent_encode(r#"var settingsFile = "/home/me/.config/battle-brothers-modern/settings.cfg";"#)));
		assert!(url.contains(&percent_encode(r#"var key = "install.data_dir";"#)));
	}

	#[test]
	fn the_pages_calls_are_read() {
		let call = |text: &str| action(&Json::parse(text).unwrap());
		assert_eq!(call(r#"["bbInstall", 0, "use", "/games/BB"]"#), Some(Action::Use("/games/BB".into())));
		assert_eq!(call(r#"["bbInstall", 0, "browse", ""]"#), Some(Action::Browse(String::new())));
		assert_eq!(call(r#"["bbInstall", 0, "quit", ""]"#), Some(Action::Quit));
		assert_eq!(call(r#"["other", 0, "quit"]"#), None);
		assert_eq!(call(r#"["bbInstall", 0]"#), None);
	}
}
