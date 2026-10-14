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
/// The on-device AI model's performance class is given as "GPU blocked" (8, what
/// Chrome for Testing assumes): otherwise, three minutes in, Chrome probes the GPU for
/// it from a utility process and logs "Unable to get gpu adapter".
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
	("optimization-guide-performance-class", Some("8")),
];

/// Chrome preferences for every page, set on CEF's UI thread once its context exists
/// (and kept in the profile, under the cache dir).
///
/// The font minimum is Coherent's WebKit's "smart minimum" (probed): font sizes
/// relative to the user default, like `body`'s 62.5% of 62.5% of `medium`, compute to
/// at least 8px, and `em` lengths use the result. Absolute sizes below it are left
/// alone. Chromium's default is 6. (`BrowserSettings::minimum_logical_font_size`
/// doesn't reach Blink; the profile preference wins.)
const PREFERENCES: &[(&str, i32)] = &[("webkit.webprefs.minimum_logical_font_size", 8)];

fn set_preferences() {
	let Some(context) = request_context_get_global_context() else {
		eprintln!("bb-ui: no global request context; preferences not set");
		return;
	};
	for (name, value) in PREFERENCES {
		let Some(mut pref) = value_create() else {
			continue;
		};
		pref.set_int(*value);
		// Not `CefString::default()`: that passes a null `error`, which CEF rejects
		// without setting anything.
		let mut error = CefString::from(" ");
		if context.set_preference(Some(&(*name).into()), Some(&mut pref), Some(&mut error)) != 1 {
			eprintln!("bb-ui: could not set preference {name}: {error}");
		}
	}
}

wrap_browser_process_handler! {
	struct BrowserReady {
		ready: Arc<Mutex<Option<mpsc::Sender<()>>>>,
	}

	impl BrowserProcessHandler {
		fn on_context_initialized(&self) {
			set_preferences();
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
