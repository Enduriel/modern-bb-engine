//! Where the update notice's "Open link" sends the player: the engine's Nexus Mods page
//! once it has one ([`NEXUS_PAGE`]), where most players get their mods; until then the
//! release's page on GitHub. Each is checked strictly: nothing else is ever opened.

/// The engine's Nexus Mods page, `https://www.nexusmods.com/battlebrothers/mods/<n>`:
/// set it once the page exists, and the notices of builds made from then on link there
/// rather than to the release on GitHub (see `docs/packaging.md`). A value that isn't
/// such a page is not used.
pub const NEXUS_PAGE: Option<&str> = None;

/// Where the engine's releases are on GitHub: a release's `html_url` must be below it.
const RELEASES: &str = "https://github.com/Enduriel/modern-bb-engine/releases/";

/// A Nexus Mods page of a Battle Brothers mod, before its number.
const NEXUS_MODS: &str = "nexusmods.com/battlebrothers/mods/";

/// The link to open: the Nexus page `nexus` if it is one, else the release's page
/// `html_url` if it is one; `None` if neither is.
pub fn choose(nexus: Option<&str>, html_url: &str) -> Option<String> {
	nexus.and_then(nexus_page).or_else(|| release_page(html_url))
}

/// `url` as the canonical address of a Battle Brothers mod's Nexus Mods page
/// (`https://www.nexusmods.com/battlebrothers/mods/<n>`), if it is one: `https`, with or
/// without `www.`, a number of up to ten digits, at most a trailing `/`, nothing else.
pub fn nexus_page(url: &str) -> Option<String> {
	let rest = url.strip_prefix("https://")?;
	let rest = rest.strip_prefix("www.").unwrap_or(rest);
	let id = rest.strip_prefix(NEXUS_MODS)?;
	let id = id.strip_suffix('/').unwrap_or(id);
	let digits = (1..=10).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_digit());
	digits.then(|| format!("https://www.{NEXUS_MODS}{id}"))
}

/// `url` if it is the page of one of the engine's releases on GitHub (below
/// [`RELEASES`], a path of letters, digits, `.`, `-`, `_` and `/`, without `..`).
pub fn release_page(url: &str) -> Option<String> {
	let path = url.strip_prefix(RELEASES)?;
	let plain = path.bytes().all(|b| b.is_ascii_alphanumeric() || b"-._/".contains(&b));
	(plain && !path.is_empty() && path.len() <= 200 && !path.contains("..")).then(|| url.to_string())
}

#[cfg(test)]
mod tests {
	use super::*;

	const RELEASE: &str = "https://github.com/Enduriel/modern-bb-engine/releases/tag/v0.3.0";

	#[test]
	fn nexus_pages_are_read_strictly() {
		let canonical = Some("https://www.nexusmods.com/battlebrothers/mods/1234".to_string());
		for url in [
			"https://www.nexusmods.com/battlebrothers/mods/1234",
			"https://nexusmods.com/battlebrothers/mods/1234",
			"https://www.nexusmods.com/battlebrothers/mods/1234/",
		] {
			assert_eq!(nexus_page(url), canonical, "{url}");
		}
		for url in [
			"http://www.nexusmods.com/battlebrothers/mods/1234",
			"https://www.nexusmods.com/skyrim/mods/1234",
			"https://www.nexusmods.com/battlebrothers/mods/",
			"https://www.nexusmods.com/battlebrothers/mods/12a",
			"https://www.nexusmods.com/battlebrothers/mods/1234?tab=files",
			"https://www.nexusmods.com/battlebrothers/mods/1234#x",
			"https://www.nexusmods.com/battlebrothers/mods/12345678901",
			"https://evil.com/www.nexusmods.com/battlebrothers/mods/1",
			"https://www.nexusmods.com.evil.com/battlebrothers/mods/1",
			"https://user@www.nexusmods.com/battlebrothers/mods/1",
			"https://www.nexusmods.com/battlebrothers/mods/1 2",
		] {
			assert_eq!(nexus_page(url), None, "{url}");
		}
	}

	#[test]
	fn the_nexus_page_once_set_is_one() {
		// What setting NEXUS_PAGE to the engine's page would give.
		let set = "https://www.nexusmods.com/battlebrothers/mods/987";
		assert_eq!(choose(Some(set), RELEASE).as_deref(), Some(set));
		// Whatever it is set to must pass, or the notices would quietly link to GitHub.
		if let Some(page) = NEXUS_PAGE {
			assert!(nexus_page(page).is_some(), "NEXUS_PAGE is no Nexus Mods page: {page}");
		}
	}

	#[test]
	fn without_a_nexus_page_the_release_on_github() {
		assert_eq!(choose(None, RELEASE).as_deref(), Some(RELEASE));
		assert_eq!(choose(Some("https://example.com/"), RELEASE).as_deref(), Some(RELEASE), "an unusable page isn't used");
		for url in [
			"https://github.com/Enduriel/modern-bb-engine/releases/",
			"https://github.com/Enduriel/other/releases/tag/v1.0.0",
			"http://github.com/Enduriel/modern-bb-engine/releases/tag/v1.0.0",
			"https://github.com/Enduriel/modern-bb-engine/releases/tag/v1.0.0?x=1",
			"https://github.com/Enduriel/modern-bb-engine/releases/../../evil",
			"https://github.com/Enduriel/modern-bb-engine/releases/tag/v1.0.0 x",
			"https://github.com.evil.com/Enduriel/modern-bb-engine/releases/tag/v1",
			"javascript:alert(1)",
			"",
		] {
			assert_eq!(release_page(url), None, "{url:?}");
			assert_eq!(choose(None, url), None, "{url:?}");
		}
	}
}
