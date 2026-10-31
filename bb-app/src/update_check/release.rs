//! GitHub's answer for a release (`GET /repos/{owner}/{repo}/releases/latest`, JSON):
//! the [`Release`] fields the check uses.

use bb_ui::json::Json;

use super::Release;

/// The release in GitHub's JSON `text`: its `tag_name`, `html_url` and `body` (the
/// notes, which may be `null`). A draft or a pre-release isn't taken (`latest` never
/// gives one).
pub fn parse(text: &str) -> Result<Release, String> {
	let json = Json::parse(text).map_err(|e| format!("the answer is not JSON: {e}"))?;
	if !matches!(json, Json::Object(_)) {
		return Err("the answer is not a JSON object".into());
	}
	let text_field = |name: &str| json.get(name).and_then(Json::as_str).map(str::to_string);
	let flag = |name: &str| json.get(name) == Some(&Json::Bool(true));
	if flag("draft") || flag("prerelease") {
		return Err("the answer is a draft or a pre-release".into());
	}
	Ok(Release {
		tag: text_field("tag_name").ok_or("the answer has no tag_name")?,
		html_url: text_field("html_url").ok_or("the answer has no html_url")?,
		notes: text_field("body").unwrap_or_default(),
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn reads_githubs_answer() {
		// Trimmed from a real answer's shape.
		let text = r###"{"url":"https://api.github.com/repos/Enduriel/modern-bb-engine/releases/1",
			"html_url":"https://github.com/Enduriel/modern-bb-engine/releases/tag/v0.3.0",
			"id":1,"tag_name":"v0.3.0","name":"0.3.0","draft":false,"prerelease":false,
			"assets":[{"name":"bb-linux.zip"}],"body":"## Changes\r\n- <b>faster</b>"}"###;
		assert_eq!(
			parse(text),
			Ok(Release {
				tag: "v0.3.0".into(),
				html_url: "https://github.com/Enduriel/modern-bb-engine/releases/tag/v0.3.0".into(),
				notes: "## Changes\r\n- <b>faster</b>".into(),
			})
		);
		let no_notes = r#"{"tag_name":"v1.0.0","html_url":"https://x","body":null}"#;
		assert_eq!(parse(no_notes).map(|r| r.notes), Ok(String::new()));
	}

	#[test]
	fn what_isnt_a_release_is_an_error() {
		for text in [
			"",
			"not json",
			"[]",
			r#"{"message":"Not Found","documentation_url":"https://docs.github.com"}"#,
			r#"{"tag_name":"v1.0.0"}"#,
			r#"{"tag_name":1,"html_url":"https://x"}"#,
			r#"{"tag_name":"v1.0.0","html_url":"https://x","draft":true}"#,
			r#"{"tag_name":"v1.0.0","html_url":"https://x","prerelease":true}"#,
		] {
			assert!(parse(text).is_err(), "{text:?}");
		}
	}
}
