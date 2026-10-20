//! Which URLs the page may have opened in the system's browser (`engine.call('openURL',
//! url)`, see [`crate::bridge`]). Any mod's JS can make that call, so only plain web
//! addresses go out: `http://` or `https://`, a host, and nothing but the characters a
//! URL may hold (RFC 3986; anything else is percent-encoded). That leaves no spaces,
//! quotes or control characters for a launcher to split or read, and no other scheme
//! (`file:`, a program's own) for the system to hand to some program.
//!
//! The one exception is Steam's store page for an app, `steam://advertise/<app id>`,
//! which vanilla's DLC links on the main menu open on a Steam build. Only that exact
//! form passes: Steam's other `steam://` commands run, install or uninstall things.
//!
//! An address without a scheme that starts `www.` opens as `http://` (vanilla's version
//! label passes `www.battlebrothersgame.com`), as Windows' shell guesses for the
//! original.

/// The longest URL opened. Browsers take more, but no link a mod shows needs it.
const MAX_LEN: usize = 4096;

/// The schemes opened, with the `//` that starts the host.
const SCHEMES: [&str; 2] = ["http://", "https://"];

/// What an address without a scheme starts with to be taken as `http://` one.
const GUESSED_WEB: &str = "www.";

/// The Steam command opened, followed by nothing but an app id.
const STEAM_STORE_PAGE: &str = "steam://advertise/";

/// The characters RFC 3986 allows in a URL besides letters and digits (unreserved,
/// reserved, and `%` for the encoded rest).
const URL_PUNCTUATION: &str = "-._~:/?#[]@!$&'()*+,;=%";

/// The URL to open for `url`, if it may be opened; if not, why not.
pub fn to_open(url: &str) -> Result<String, &'static str> {
	let guessed = url
		.get(..GUESSED_WEB.len())
		.is_some_and(|head| head.eq_ignore_ascii_case(GUESSED_WEB));
	let url = if guessed { format!("http://{url}") } else { url.to_string() };
	if !is_steam_store_page(&url) {
		check(&url)?;
	}
	Ok(url)
}

/// Whether `url` is `steam://advertise/` and an app id (one to ten digits).
fn is_steam_store_page(url: &str) -> bool {
	url.get(..STEAM_STORE_PAGE.len())
		.is_some_and(|head| head.eq_ignore_ascii_case(STEAM_STORE_PAGE))
		&& url
			.get(STEAM_STORE_PAGE.len()..)
			.is_some_and(|id| (1..=10).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_digit()))
}

/// Whether `url` may be opened as it is; if not, why not.
fn check(url: &str) -> Result<(), &'static str> {
	if url.len() > MAX_LEN {
		return Err("too long");
	}
	let rest = SCHEMES
		.iter()
		.find_map(|scheme| {
			url.get(..scheme.len())
				.filter(|head| head.eq_ignore_ascii_case(scheme))
				.map(|_| &url[scheme.len()..])
		})
		.ok_or("not an http(s) URL")?;
	if !url
		.chars()
		.all(|c| c.is_ascii_alphanumeric() || URL_PUNCTUATION.contains(c))
	{
		return Err("a character a URL can't hold");
	}
	// The host: the authority up to the path, query or fragment, past any user info,
	// before any port.
	let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
	let host = authority.rsplit('@').next().unwrap_or("");
	let host = match host.strip_prefix('[') {
		Some(ipv6) => ipv6.split(']').next().unwrap_or(""),
		None => host.split(':').next().unwrap_or(""),
	};
	if host.is_empty() {
		return Err("no host");
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::{check, to_open};

	#[test]
	fn web_addresses_pass() {
		for url in [
			"https://www.nexusmods.com/battlebrothers/mods/765",
			"http://battlebrothersgame.com",
			"https://www.youtube.com/playlist?list=PLMbw15ySmbBtpEj0segi1DzKu2f_Yqr-W",
			"HTTPS://Example.com/a%20b?x=1&y=(2)#top",
			"https://user@host:8080/",
			"https://[::1]:8080/x",
			"https://github.com/MSUTeam/MSU/releases/tag/1.2.3",
		] {
			assert_eq!(check(url), Ok(()), "{url}");
		}
	}

	#[test]
	fn other_schemes_are_refused() {
		for url in [
			"file:///etc/passwd",
			"javascript:alert(1)",
			"ftp://example.com",
			"mailto:a@b.c",
			"C:\\Windows\\system32\\calc.exe",
			"/usr/bin/xterm",
			"-e xterm",
			"--help",
			"https:example.com",
			"http:/example.com",
			"",
		] {
			assert_eq!(check(url), Err("not an http(s) URL"), "{url:?}");
		}
	}

	#[test]
	fn characters_a_launcher_could_misread_are_refused() {
		for url in [
			"https://example.com/a b",
			"https://example.com/\t",
			"https://example.com/\nfile:///x",
			"https://example.com/\0",
			"https://example.com/\u{7f}",
			"https://example.com/\"&calc",
			"https://example.com/a\\b",
			"https://example.com/<x>",
			"https://example.com/^|`{}",
			"https://exämple.com/",
			"https://example.com/\u{2028}",
		] {
			assert_eq!(check(url), Err("a character a URL can't hold"), "{url:?}");
		}
	}

	#[test]
	fn a_host_is_needed() {
		for url in [
			"https://",
			"https:///path",
			"http://?q",
			"https://user@/x",
			"https://:80/",
			"https://[]/",
		] {
			assert_eq!(check(url), Err("no host"), "{url:?}");
		}
	}

	#[test]
	fn www_addresses_open_as_http() {
		assert_eq!(
			to_open("www.battlebrothersgame.com").as_deref(),
			Ok("http://www.battlebrothersgame.com")
		);
		assert_eq!(to_open("WWW.example.com/x?y").as_deref(), Ok("http://WWW.example.com/x?y"));
		assert_eq!(
			to_open("https://www.nexusmods.com/battlebrothers/mods/765").as_deref(),
			Ok("https://www.nexusmods.com/battlebrothers/mods/765")
		);
		assert_eq!(to_open("example.com"), Err("not an http(s) URL"));
		assert_eq!(to_open("www. x"), Err("a character a URL can't hold"));
		assert_eq!(to_open("www./x"), Ok("http://www./x".to_string()));
	}

	#[test]
	fn steam_store_pages_open_and_nothing_else_of_steam() {
		for url in ["steam://advertise/1361280", "steam://advertise/732460", "STEAM://Advertise/1"] {
			assert_eq!(to_open(url).as_deref(), Ok(url), "{url}");
		}
		for url in [
			"steam://run/365360",
			"steam://rungameid/365360",
			"steam://uninstall/365360",
			"steam://advertise/",
			"steam://advertise/12345678901",
			"steam://advertise/1361280/x",
			"steam://advertise/1361280 -x",
			"steam://advertise/-1",
			"steam://advertise/../run/1",
		] {
			assert!(to_open(url).is_err(), "{url}");
		}
	}

	#[test]
	fn overlong_urls_are_refused() {
		let url = format!("https://example.com/{}", "a".repeat(4096));
		assert_eq!(check(&url), Err("too long"));
	}
}
