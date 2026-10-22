//! The system's folder dialog, opened through the page's browser (CEF's
//! `RunFileDialog`): Windows' and macOS's own; on Linux, the desktop's file chooser
//! portal if the session has one, else GTK's dialog. The choice comes back as an
//! [`Event::FolderChosen`] once the player closes it.

use std::path::PathBuf;
use std::sync::Arc;

use cef::*;

use super::UiHost;
use super::client::{Event, Shared};

wrap_run_file_dialog_callback! {
	struct Chosen {
		shared: Arc<Shared>,
	}

	impl RunFileDialogCallback {
		fn on_file_dialog_dismissed(&self, file_paths: Option<&mut CefStringList>) {
			let first = file_paths.and_then(|paths| paths.clone().into_iter().next());
			self.shared.send(Event::FolderChosen(first.filter(|p| !p.is_empty()).map(PathBuf::from)));
		}
	}
}

impl UiHost {
	/// Open the system's folder dialog, titled `title`, at `start` if given; its answer
	/// comes as an [`Event::FolderChosen`]. One at a time: CEF answers a second with
	/// `None` while the first is open. Without the page's browser, the answer is `None`
	/// at once.
	pub fn choose_folder(&self, title: &str, start: Option<&std::path::Path>) {
		let Some(host) = self.browser_host() else {
			self.shared.send(Event::FolderChosen(None));
			return;
		};
		let mut callback = Chosen::new(self.shared.clone());
		let start = start.map(|p| CefString::from(p.to_string_lossy().as_ref()));
		// No filters, but a list all the same: CEF reads it unchecked (a null one crashed).
		let mut filters = CefStringList::new();
		host.run_file_dialog(
			FileDialogMode::OPEN_FOLDER,
			Some(&title.into()),
			start.as_ref(),
			Some(&mut filters),
			Some(&mut callback),
		);
	}
}
