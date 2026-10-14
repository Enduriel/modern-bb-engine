//! The settings file's format: one `key = value` a line, `#` starting a comment line.
//! Reading is tolerant (a line without `=` is kept as it is, a key read twice takes
//! the last value), and writing keeps every line it doesn't change, so options a later
//! version adds, and the player's comments, survive an older one's save.

/// A settings file's lines, in order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SettingsText {
	lines: Vec<Line>,
}

#[derive(Clone, Debug, PartialEq)]
enum Line {
	Entry { key: String, value: String },
	/// A comment, a blank line, or one that isn't `key = value`, kept as it was.
	Other(String),
}

impl SettingsText {
	pub fn parse(text: &str) -> SettingsText {
		let lines = text
			.lines()
			.map(|line| match line.split_once('=') {
				Some((key, value)) if !line.trim_start().starts_with('#') && !key.trim().is_empty() => Line::Entry {
					key: key.trim().to_string(),
					value: value.trim().to_string(),
				},
				_ => Line::Other(line.to_string()),
			})
			.collect();
		SettingsText { lines }
	}

	/// The value of `key`, the last one if it's there twice.
	pub fn get(&self, key: &str) -> Option<&str> {
		self.lines.iter().rev().find_map(|line| match line {
			Line::Entry { key: k, value } if k == key => Some(value.as_str()),
			_ => None,
		})
	}

	/// Set `key` to `value`: in place where it is (all of its lines), at the end if it isn't.
	pub fn set(&mut self, key: &str, value: &str) {
		let mut found = false;
		for line in &mut self.lines {
			if let Line::Entry { key: k, value: v } = line {
				if k == key {
					*v = value.to_string();
					found = true;
				}
			}
		}
		if !found {
			self.lines.push(Line::Entry {
				key: key.to_string(),
				value: value.to_string(),
			});
		}
	}

	pub fn render(&self) -> String {
		let mut text = String::new();
		for line in &self.lines {
			match line {
				Line::Entry { key, value } => text += &format!("{key} = {value}"),
				Line::Other(other) => text += other,
			}
			text.push('\n');
		}
		text
	}
}

#[cfg(test)]
mod tests {
	use super::SettingsText;

	#[test]
	fn reads_entries_and_keeps_the_rest() {
		let text = "# mine\nvideo.width = 1280\n\nnot an entry\nfuture.option=on\nvideo.width=1600\n";
		let mut s = SettingsText::parse(text);
		assert_eq!(s.get("video.width"), Some("1600"), "the last wins");
		assert_eq!(s.get("future.option"), Some("on"));
		assert_eq!(s.get("missing"), None);
		s.set("video.width", "1920");
		s.set("video.height", "1080");
		assert_eq!(
			s.render(),
			"# mine\nvideo.width = 1920\n\nnot an entry\nfuture.option = on\nvideo.width = 1920\nvideo.height = 1080\n"
		);
		assert_eq!(SettingsText::parse(&s.render()), s, "a round trip");
	}

	#[test]
	fn a_commented_entry_is_a_comment() {
		let s = SettingsText::parse("# video.width = 1\n = 2\n");
		assert_eq!(s.get("video.width"), None);
		assert_eq!(s.get(""), None);
		assert_eq!(s.render(), "# video.width = 1\n = 2\n");
	}
}
