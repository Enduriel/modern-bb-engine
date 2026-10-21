//! The CEF application object, shared by the browser process and its children: the
//! `coui` scheme registration (every process must agree on it), command-line
//! switches, and the per-process handlers.

use std::sync::{Arc, Mutex, mpsc};

use cef::*;

use super::bridge_client;
use super::renderer::PageHandler;
use super::scheme::scheme_options;
use crate::coui::SCHEME;

/// Switches for the browser process (children inherit what they need), besides the
/// display connection's platform ([`Display`](super::Display), `--ozone-platform`).
///
/// Offscreen, composited on the GPU: Chromium's GPU process composites the page and
/// hands each paint over as a GPU buffer (`shared_texture_enabled`,
/// `OnAcceleratedPaint`), never as pixels in memory. ANGLE runs on the system's EGL
/// (`gl-egl`, what CEF picks for offscreen GPU paints on Linux). The page's content is
/// still rasterized on the CPU (`--disable-gpu-rasterization`, added with these): GPU
/// raster changes pixels against the original's, e.g. the battle results' name
/// separators (`line_metal.png` as a bottom `border-image`) came out a light 3 px bar
/// where the original and CPU raster draw dark, light, dark rows; CPU raster keeps the
/// frame goldens exact for ~3% of a core more in animations. `BB_UI_GPU_RASTER=1` rasterizes on the GPU instead, to compare. No audio output, and
/// none of Chrome's background services. The on-device AI model's performance class is
/// given as "GPU blocked" (8, what Chrome for Testing assumes): otherwise, three minutes
/// in, Chrome probes the GPU for it from a utility process and logs "Unable to get gpu
/// adapter".
///
/// As few processes as the page allows, so the game shows as one program: the GPU
/// process and the network service run inside the game's, there is no zygote (there is
/// no sandbox for it to prepare) and no spare renderer. Left beside the game: the page's
/// renderer and the storage service. Measured on the idle world map: 8 processes and
/// 772 MB (PSS) before, 3 and 742 MB after. The renderer stays out of the game's
/// process: `--single-process` isn't supported upstream, and our page's handlers assume
/// their own process.
const SWITCHES: &[(&str, Option<&str>)] = &[
	("use-angle", Some("gl-egl")),
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

/// V8's flags for the page (`--js-flags`), unless `BB_UI_V8_FLAGS` gives others (empty:
/// none): its memory saver mode, which keeps the heap small at the cost of more frequent
/// collections. In V8 15.4 (Chromium 154)
/// `--optimize-for-size` does two things: a young generation of 1 MB semi-spaces (32 MB
/// by default on desktop), and `MemorySaverModeEnabled`, under which the old generation
/// and the embedder's (Blink's DOM, Oilpan) grow by at most 1.3x the live size between
/// full collections (up to 4x by default). The page's DOM churns in battles (the
/// combat log, tooltips, the turn bar) while its JavaScript heap stays small, so by
/// default those collections came rarely and the renderer grew by ~50 MB a battle until
/// the battle's end released it; with the flag it stays flat (-45 MB at the busiest),
/// for no measurable frame or CPU cost. Recheck on a V8 upgrade: the flag's meaning
/// may change.
const V8_FLAGS: &str = "--optimize-for-size";

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
		// The bridge's endpoint (`bridge_wire`), for the renderer's command line only.
		bridge: Option<String>,
	}

	impl BrowserProcessHandler {
		fn on_before_child_process_launch(&self, command_line: Option<&mut CommandLine>) {
			let (Some(command_line), Some(bridge)) = (command_line, &self.bridge) else { return };
			let kind = CefString::from(&command_line.switch_value(Some(&"type".into()))).to_string();
			if kind == "renderer" {
				command_line.append_switch_with_value(Some(&bridge_client::SWITCH.into()), Some(&bridge.as_str().into()));
			}
		}

		// Only with `external_message_pump` (macOS): when to run CEF's work.
		fn on_schedule_message_pump_work(&self, delay_ms: i64) {
			super::pump_schedule::SCHEDULE.schedule(delay_ms);
		}

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
		bridge: Option<String>,
		// `--ozone-platform` (the browser process's display connection).
		ozone: Option<&'static str>,
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
			if let Some(ozone) = self.ozone {
				command_line.append_switch_with_value(Some(&"ozone-platform".into()), Some(&ozone.into()));
			}
			for (name, value) in SWITCHES {
				match value {
					Some(value) => command_line.append_switch_with_value(Some(&(*name).into()), Some(&(*value).into())),
					None => command_line.append_switch(Some(&(*name).into())),
				}
			}
			// macOS: Chromium keeps its cookie key in the user's keychain; the page needs no
			// secrets, so it stays out of it.
			#[cfg(target_os = "macos")]
			command_line.append_switch(Some(&"use-mock-keychain".into()));
			// The page's pixels rasterized on the CPU, as before (see `SWITCHES`), unless
			// asked to compare with Chromium's GPU raster.
			if std::env::var_os("BB_UI_GPU_RASTER").is_none_or(|v| v == "0") {
				command_line.append_switch(Some(&"disable-gpu-rasterization".into()));
			}
			let v8_flags = std::env::var("BB_UI_V8_FLAGS").unwrap_or_else(|_| V8_FLAGS.to_string());
			if !v8_flags.trim().is_empty() {
				command_line.append_switch_with_value(Some(&"js-flags".into()), Some(&v8_flags.as_str().into()));
			}
			// More switches, to compare Chromium's settings: `BB_UI_SWITCHES="name=value name2"`
			// (features join the lists above; V8's flags too, one `js-flags=--flag` each).
			for switch in std::env::var("BB_UI_SWITCHES").unwrap_or_default().split_whitespace() {
				let switch = switch.trim_start_matches('-');
				match switch.split_once('=') {
					Some((name, value)) => {
						let old = CefString::from(&command_line.switch_value(Some(&name.into()))).to_string();
						let value = match name {
							"enable-features" | "disable-features" if !old.is_empty() => format!("{old},{value}"),
							"js-flags" if !old.is_empty() => format!("{old} {value}"),
							_ => value.to_string(),
						};
						command_line.append_switch_with_value(Some(&name.into()), Some(&value.as_str().into()));
					}
					None => command_line.append_switch(Some(&switch.into())),
				}
			}
			// A renderer in this process (`--single-process`, to compare) reads the bridge's
			// endpoint from this command line.
			if let Some(bridge) = &self.bridge
				&& command_line.has_switch(Some(&"single-process".into())) == 1
			{
				command_line.append_switch_with_value(Some(&bridge_client::SWITCH.into()), Some(&bridge.as_str().into()));
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
			Some(BrowserReady::new(self.ready.clone(), self.bridge.clone()))
		}

		fn render_process_handler(&self) -> Option<RenderProcessHandler> {
			Some(PageHandler::new())
		}
	}
}

/// The application object; `ready` (browser process only) is signalled once CEF's
/// context is initialized. `devtools_port` for the browser process when DevTools is
/// served, `ozone` its display connection's platform (its children don't take switches
/// from it). `bridge` is the bridge's endpoint, which it hands the renderer.
pub(super) fn host_app(
	ready: Option<mpsc::Sender<()>>,
	devtools_port: Option<u16>,
	bridge: Option<String>,
	ozone: Option<&'static str>,
) -> App {
	HostApp::new(Arc::new(Mutex::new(ready)), devtools_port, bridge, ozone)
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
