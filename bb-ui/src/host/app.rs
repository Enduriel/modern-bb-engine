//! The CEF application object, shared by the browser process and its children: the
//! `coui` scheme registration (every process must agree on it), command-line
//! switches, and the per-process handlers.

use std::sync::{Arc, Mutex, mpsc};

use cef::*;

use super::renderer::PageHandler;
use super::scheme::scheme_options;
use crate::coui::SCHEME;

/// Switches for the browser process (children inherit what they need).
///
/// Offscreen and headless: no display connection, software compositing (frames
/// arrive in `on_paint`), no audio output, and none of Chrome's background services.
const SWITCHES: &[(&str, Option<&str>)] = &[
	("ozone-platform", Some("headless")),
	("disable-gpu", None),
	("disable-gpu-compositing", None),
	("mute-audio", None),
	("no-first-run", None),
	("disable-extensions", None),
	("disable-component-update", None),
	("disable-background-networking", None),
	("disable-sync", None),
	("disable-default-apps", None),
	("password-store", Some("basic")),
	("force-device-scale-factor", Some("1")),
];

wrap_browser_process_handler! {
	struct BrowserReady {
		ready: Arc<Mutex<Option<mpsc::Sender<()>>>>,
	}

	impl BrowserProcessHandler {
		fn on_context_initialized(&self) {
			if let Some(ready) = self.ready.lock().unwrap_or_else(|e| e.into_inner()).take() {
				let _ = ready.send(());
			}
		}
	}
}

wrap_app! {
	pub(super) struct HostApp {
		ready: Arc<Mutex<Option<mpsc::Sender<()>>>>,
	}

	impl App {
		fn on_before_command_line_processing(
			&self,
			process_type: Option<&CefString>,
			command_line: Option<&mut CommandLine>,
		) {
			let browser_process = process_type.is_none_or(|t| t.to_string().is_empty());
			let Some(command_line) = command_line else { return };
			if !browser_process {
				return;
			}
			for (name, value) in SWITCHES {
				match value {
					Some(value) => command_line.append_switch_with_value(Some(&(*name).into()), Some(&(*value).into())),
					None => command_line.append_switch(Some(&(*name).into())),
				}
			}
		}

		fn on_register_custom_schemes(&self, registrar: Option<&mut SchemeRegistrar>) {
			if let Some(registrar) = registrar {
				registrar.add_custom_scheme(Some(&SCHEME.into()), scheme_options());
			}
		}

		fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
			Some(BrowserReady::new(self.ready.clone()))
		}

		fn render_process_handler(&self) -> Option<RenderProcessHandler> {
			Some(PageHandler::new())
		}
	}
}

/// The application object; `ready` (browser process only) is signalled once CEF's
/// context is initialized.
pub(super) fn host_app(ready: Option<mpsc::Sender<()>>) -> App {
	HostApp::new(Arc::new(Mutex::new(ready)))
}
