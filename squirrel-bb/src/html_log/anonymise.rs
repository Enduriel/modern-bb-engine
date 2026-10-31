//! Paths in log.html without the player's name ([`Anonymiser`]): the player's home
//! folder, wherever a path in a row starts with it, is written as a placeholder
//! (`%USERPROFILE%` on Windows, `~` elsewhere), and so is any other folder given its own
//! (the Documents folder where Windows has it moved off the home folder, to another
//! drive). The original logs the full path of its folder in `Documents`, and a Windows
//! user name is often a real name; logs are made to be shared.
//!
//! The log's writer applies it to every row, so no line has to remember to. A folder is
//! matched as a path, not as text:
//! - only whole: the home `C:\Users\Al` is not the start of `C:\Users\Alice`;
//! - on Windows with either separator (`\` or `/`), and without regard to ASCII case;
//! - a separator may be repeated (`C:\\Users\\Al`, as an escaped string shows it);
//! - any of its bytes may be percent-encoded (`file:///C:/Users/Jan%20Kowalski/`).
//!
//! A home that is no folder of its own (unset, empty, relative, a root such as `/` or
//! `C:\`) is never matched: it would turn every path into a placeholder. The user's name
//! anywhere else (outside a path to the home folder) stays as it is.

use std::borrow::Cow;
use std::path::Path;

/// The rules of a system's paths.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
	/// Both `\` and `/` separate, ASCII case doesn't matter.
	Windows,
	/// Only `/` separates; case matters.
	Unix,
}

impl Style {
	/// This system's.
	pub const HOST: Style = if cfg!(windows) { Style::Windows } else { Style::Unix };

	/// The home folder's placeholder.
	pub fn home_placeholder(self) -> &'static str {
		match self {
			Style::Windows => "%USERPROFILE%",
			Style::Unix => "~",
		}
	}

	fn is_separator(self, b: u8) -> bool {
		b == b'/' || (self == Style::Windows && b == b'\\')
	}

	fn same(self, a: u8, b: u8) -> bool {
		match self {
			Style::Windows => a.eq_ignore_ascii_case(&b),
			Style::Unix => a == b,
		}
	}
}

/// A folder hidden behind its placeholder.
struct Folder {
	/// Its path, without trailing separators.
	path: Vec<u8>,
	placeholder: String,
}

/// The folders hidden in a log's rows, and the replacing.
pub struct Anonymiser {
	style: Style,
	/// Longest first, so a folder inside another is matched before it.
	folders: Vec<Folder>,
}

impl Anonymiser {
	/// One hiding nothing yet, for paths of `style`.
	pub fn new(style: Style) -> Anonymiser {
		Anonymiser { style, folders: Vec::new() }
	}

	/// This system's, hiding the user's home folder (the system's: `USERPROFILE` or the
	/// user's profile on Windows, `HOME` or the user's entry elsewhere).
	pub fn for_this_user() -> Anonymiser {
		let mut anonymiser = Anonymiser::new(Style::HOST);
		if let Some(home) = std::env::home_dir() {
			anonymiser.hide(&home, Style::HOST.home_placeholder());
		}
		anonymiser
	}

	/// Hide `folder` behind `placeholder`, unless it is no folder of its own (see the
	/// module's notes) or is already inside a hidden folder (Documents in the home folder,
	/// OneDrive's included, stays `%USERPROFILE%\…\Documents`). Whether it was added.
	pub fn hide(&mut self, folder: &Path, placeholder: &str) -> bool {
		self.hide_bytes(folder.as_os_str().as_encoded_bytes(), placeholder)
	}

	/// [`hide`](Self::hide) for a path as bytes (UTF-8, as the log's rows have it).
	pub fn hide_bytes(&mut self, folder: &[u8], placeholder: &str) -> bool {
		let Some(path) = own_folder(folder, self.style) else { return false };
		if matches!(self.apply(path), Cow::Owned(_)) {
			return false;
		}
		self.folders.push(Folder { path: path.to_vec(), placeholder: placeholder.to_string() });
		self.folders.sort_by_key(|f| std::cmp::Reverse(f.path.len()));
		true
	}

	/// `text` with each hidden folder's paths starting with its placeholder.
	pub fn apply<'a>(&self, text: &'a [u8]) -> Cow<'a, [u8]> {
		if self.folders.is_empty() {
			return Cow::Borrowed(text);
		}
		let mut out: Option<Vec<u8>> = None;
		let mut i = 0;
		while i < text.len() {
			let found = self.folders.iter().find_map(|f| self.match_at(&f.path, text, i).map(|end| (f, end)));
			match found {
				Some((folder, end)) => {
					let out = out.get_or_insert_with(|| text[..i].to_vec());
					out.extend_from_slice(folder.placeholder.as_bytes());
					i = end;
				}
				None => {
					if let Some(out) = out.as_mut() {
						out.push(text[i]);
					}
					i += 1;
				}
			}
		}
		match out {
			Some(out) => Cow::Owned(out),
			None => Cow::Borrowed(text),
		}
	}

	/// Where the folder `path` ends if a path to it (or inside it) starts at `text[at..]`.
	fn match_at(&self, path: &[u8], text: &[u8], at: usize) -> Option<usize> {
		let style = self.style;
		let (mut j, mut p) = (0, at);
		// Its leading separators one for one (`/home` in `file:///home` is the last `/`'s).
		while j < path.len() && style.is_separator(path[j]) {
			p += self.separator_at(text, p)?;
			j += 1;
		}
		while j < path.len() {
			if style.is_separator(path[j]) {
				let start = p;
				while let Some(len) = self.separator_at(text, p) {
					p += len;
				}
				if p == start {
					return None;
				}
				while j < path.len() && style.is_separator(path[j]) {
					j += 1;
				}
				continue;
			}
			p += byte_at(text, p, |b| style.same(b, path[j]))?;
			j += 1;
		}
		self.ends_folder(text, p).then_some(p)
	}

	/// The length of a separator at `text[p..]` (as it is, or percent-encoded).
	fn separator_at(&self, text: &[u8], p: usize) -> Option<usize> {
		byte_at(text, p, |b| self.style.is_separator(b))
	}

	/// Whether a folder's name ends at `text[p]`: the text's end, a separator, or what
	/// can't go on a file's name (a space, a quote, markup, `:`, `,`, `;`, …). A `.` ends
	/// it only before the text's end or a space (a sentence's end).
	fn ends_folder(&self, text: &[u8], p: usize) -> bool {
		let Some(&b) = text.get(p) else { return true };
		if self.separator_at(text, p).is_some() {
			return true;
		}
		match b {
			b'.' => text.get(p + 1).is_none_or(|c| c.is_ascii_whitespace()),
			b'"' | b'\'' | b'<' | b'>' | b'|' | b'?' | b'*' | b':' | b',' | b';' | b')' | b']' => true,
			_ => b.is_ascii_whitespace(),
		}
	}
}

/// The length of a byte at `text[p..]` that `want` takes: as it is, or as `%HH`.
fn byte_at(text: &[u8], p: usize, want: impl Fn(u8) -> bool) -> Option<usize> {
	let &b = text.get(p)?;
	if want(b) {
		return Some(1);
	}
	if b == b'%' {
		let hex = text.get(p + 1..p + 3)?;
		let hex = std::str::from_utf8(hex).ok()?;
		let decoded = u8::from_str_radix(hex, 16).ok()?;
		if want(decoded) {
			return Some(3);
		}
	}
	None
}

/// `folder` without its trailing separators, if it is an absolute path below a root:
/// `/x` on Unix; `C:\x` or `\\server\share` on Windows (a verbatim `\\?\C:\x` as `C:\x`).
fn own_folder(folder: &[u8], style: Style) -> Option<&[u8]> {
	let mut end = folder.len();
	while end > 0 && style.is_separator(folder[end - 1]) {
		end -= 1;
	}
	let path = &folder[..end];
	match style {
		Style::Unix => (path.len() > 1 && path[0] == b'/').then_some(path),
		Style::Windows => {
			// A device path (`\\?\`, `\\.\`) only as a drive's.
			if let Some(rest) = path.strip_prefix(br"\\?\").or_else(|| path.strip_prefix(br"\\.\")) {
				return is_drive_path(rest).then_some(rest);
			}
			let unc = path.len() > 2 && path[..2].iter().all(|&b| style.is_separator(b)) && {
				let names = path[2..].split(|&b| style.is_separator(b)).filter(|n| !n.is_empty());
				names.count() >= 2
			};
			(is_drive_path(path) || unc).then_some(path)
		}
	}
}

/// `C:\x`: a drive's root and a name below it.
fn is_drive_path(path: &[u8]) -> bool {
	path.len() > 3
		&& path[0].is_ascii_alphabetic()
		&& path[1] == b':'
		&& Style::Windows.is_separator(path[2])
		&& path[3..].iter().any(|&b| !Style::Windows.is_separator(b))
}

#[cfg(test)]
mod tests;
