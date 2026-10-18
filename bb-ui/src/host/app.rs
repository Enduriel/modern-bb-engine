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
///
/// As few processes as the page allows, so the game shows as one program: the GPU
/// process and the network service run inside the game's (frames are composited in
/// software anyway), there is no zygote (there is no sandbox for it to prepare) and no
/// spare renderer. Left beside the game: the page's renderer and the storage service.
/// Measured on the idle world map: 8 processes and 772 MB (PSS) before, 3 and 742 MB
/// after. The renderer stays out of the game's process: `--single-process` isn't
/// supported upstream, and our page's handlers assume their own process.
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
	("optimization-guide-performance-class", Some("8")),
	("in-process-gpu", None),
	("no-zygote", None),
	("enable-features", Some("NetworkServiceInProcess2")),
	("disable-features", Some("SpareRendererForSitePerProcess")),
];

/// The origins allowed to drive the page over DevTools at `port`
/// (`--remote-allow-origins`; Chromium refuses a DevTools WebSocket from any other
/// origin): the frontend the port itself serves (`/devtools/inspector.html`, by either
/// name of the local host) and the browser's own (`devtools://devtools`, which
/// `chrome://inspect` opens). Never `*`: any website open in the player's browser could
/// then drive the game's page over localhost.
fn devtools_origins(port: u16) -> String {
	format!("http://127.0.0.1:{port},http://localhost:{port},devtools://devtools")
}

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
		// The DevTools port, if served.
		devtools_port: Option<u16>,
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
			if let Some(port) = self.devtools_port {
				let origins = devtools_origins(port);
				command_line.append_switch_with_value(Some(&"remote-allow-origins".into()), Some(&origins.as_str().into()));
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
/// The application object; `devtools_port` for the browser process when DevTools is
/// served (its children don't take switches from it).
pub(super) fn host_app(ready: Option<mpsc::Sender<()>>, devtools_port: Option<u16>) -> App {
	HostApp::new(Arc::new(Mutex::new(ready)), devtools_port)
}

#[cfg(test)]
mod tests {
	use super::devtools_origins;

	#[test]
	fn devtools_allows_only_its_own_frontends() {
		assert_eq!(devtools_origins(9222), "http://127.0.0.1:9222,http://localhost:9222,devtools://devtools");
		assert!(!devtools_origins(9222).contains('*'), "no website may drive the page");
	}
}
