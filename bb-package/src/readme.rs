//! The player's documents in a package: the README at the package's top, beside `bb`
//! (`bb.app` on macOS), and the documents it links to (`docs/`, the license), so its links work in
//! the unpacked folder as they do in the repository. Built into `bb-package`, so a
//! package always carries the README of the code it was built from.

use std::fs;
use std::path::Path;

/// Each document's path in the package (the same as in the repository) and its text.
const DOCUMENTS: &[(&str, &str)] = &[
	("README.md", include_str!("../../README.md")),
	("docs/requirements.md", include_str!("../../docs/requirements.md")),
	("docs/differences.md", include_str!("../../docs/differences.md")),
	("docs/modding.md", include_str!("../../docs/modding.md")),
	("docs/memory-snapshots.md", include_str!("../../docs/memory-snapshots.md")),
	("docs/environment.md", include_str!("../../docs/environment.md")),
	("docs/packaging.md", include_str!("../../docs/packaging.md")),
	("LICENSE", include_str!("../../LICENSE")),
];

/// Write the documents into the package folder `dir`.
pub fn write(dir: &Path) -> Result<(), String> {
	for (path, text) in DOCUMENTS {
		let to = dir.join(path);
		if let Some(parent) = to.parent() {
			fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
		}
		fs::write(&to, text).map_err(|e| format!("{}: {e}", to.display()))?;
	}
	Ok(())
}

/// The relative links (`](target)`, without a `#fragment`) in `text`; web addresses and
/// links within the same document left out.
#[cfg(test)]
fn relative_links(text: &str) -> Vec<&str> {
	text.split("](")
		.skip(1)
		.filter_map(|rest| rest.split(')').next())
		.map(|link| link.split('#').next().unwrap_or(""))
		.filter(|link| !link.is_empty() && !link.contains("://") && !link.starts_with("mailto:"))
		.collect()
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Every relative link in a shipped document names another shipped document, so no
	/// link in the unpacked package leads nowhere.
	#[test]
	fn every_link_in_the_package_resolves_in_it() {
		let shipped: Vec<&str> = DOCUMENTS.iter().map(|(path, _)| *path).collect();
		for (path, text) in DOCUMENTS {
			let folder = Path::new(path).parent().unwrap_or(Path::new(""));
			for link in relative_links(text) {
				let target = folder.join(link);
				let target = target.to_str().unwrap();
				assert!(shipped.contains(&target), "{path} links to {link}, which the package doesn't ship");
			}
		}
	}

	#[test]
	fn the_documents_are_laid_out_beside_bb() {
		let dir = std::env::temp_dir().join(format!("bb-package-readme-{}", std::process::id()));
		let _ = fs::remove_dir_all(&dir);
		write(&dir).unwrap();
		assert!(fs::read_to_string(dir.join("README.md")).unwrap().starts_with("# "));
		for (path, text) in DOCUMENTS {
			assert_eq!(fs::read_to_string(dir.join(path)).unwrap(), *text);
		}
		fs::remove_dir_all(&dir).unwrap();
	}

	#[test]
	fn relative_links_skip_web_addresses_and_fragments() {
		let text = "[a](docs/a.md) [b](https://x.org/b.md) [c](#here) [d](d.md#there)";
		assert_eq!(relative_links(text), ["docs/a.md", "d.md"]);
	}
}
