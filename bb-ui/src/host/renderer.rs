//! The renderer-process half of the host: installs the Coherent-compatible
//! `window.engine` into the page and runs the game's calls in it.
//!
//! This code runs in Chromium's renderer process (a child process re-executing the
//! game binary), on the renderer main thread. The page's calls into the game go
//! through `__bbRequest` ([`bridge_client`](super::bridge_client)), which blocks the
//! page's thread until the game answers, as the original's inline calls did.

use cef::*;

use super::ipc::{self, Arg};

/// The JS installed before any page script, in order: what the page has to do (its
/// timers and animation frames counted, for the game to tell when it may rest), the
/// bridge (see the file for the protocol), the order inserted scripts run in, the emulation of Coherent's layout and
/// style differences, scroll bars that follow their content instead of polling it, the
/// emulation of how it draws tiled backgrounds, its image cache, then the unbinding of
/// handlers the UI's plugins leave behind.
const SCRIPTS: [(&str, &str); 8] = [
	("bb://page_work.js", include_str!("page_work.js")),
	("bb://bootstrap.js", include_str!("bootstrap.js")),
	("bb://script_order.js", include_str!("script_order.js")),
	("bb://compat.js", include_str!("compat.js")),
	("bb://scroll_bars.js", include_str!("scroll_bars.js")),
	("bb://tiles.js", include_str!("tiles.js")),
	("bb://image_cache.js", include_str!("image_cache.js")),
	("bb://teardown.js", include_str!("teardown.js")),
];

/// `window.ModernEngine`: what mods' JS detects this engine by, as their Squirrel does by
/// `::ModernEngine` (`docs/modding.md`). Its `Version` is the Squirrel table's; the
/// original's page has no such global (none of the game's or the known mods' scripts
/// define one).
fn modern_engine_script() -> String {
	format!(
		"window.ModernEngine = Object.freeze({{ Version: {} }});",
		squirrel_bb::modern_engine::VERSION
	)
}

/// Call `window.<function>(argument)` in `frame`'s context; its string result.
fn call_page(frame: &Frame, function: &str, argument: &str) -> Option<String> {
	let context = frame.v8_context()?;
	if context.enter() == 0 {
		return None;
	}
	let result = (|| {
		let global = context.global()?;
		let func = global.value_bykey(Some(&function.into()))?;
		if func.is_function() == 0 {
			return None;
		}
		let arg = v8_value_create_string(Some(&argument.into()));
		let value = func.execute_function(None, Some(&[arg]))?;
		(value.is_string() != 0).then(|| CefString::from(&value.string_value()).to_string())
	})();
	context.exit();
	result
}

/// Park the page until the game lets it go on, running the game's calls into it
/// meanwhile (`__bbPark` in bootstrap.js takes each step). Each call runs from here as
/// a script call of its own, not nested inside the park's, as a call into a page that
/// isn't parked does: the promise callbacks it queues run as it returns, so what they
/// change or a timer they set is counted when the page next says what it has to do.
/// Nested in the park, they waited until the page was let go on, which a resting page
/// (`page_rest`) may not be until something else wakes it.
fn park(frame: &Frame) -> Option<String> {
	let mut next = call_page(frame, "__bbPark", "")?;
	while !next.is_empty() {
		let result = call_page(frame, "__bbCall", &next).unwrap_or_else(|| "null".into());
		next = call_page(frame, "__bbPark", &result).unwrap_or_default();
	}
	Some("unparked".into())
}

wrap_render_process_handler! {
	pub(super) struct PageHandler {}

	impl RenderProcessHandler {
		fn on_context_created(
			&self,
			_browser: Option<&mut Browser>,
			frame: Option<&mut Frame>,
			context: Option<&mut V8Context>,
		) {
			// Only the page itself is the UI; frames inside it (e.g. a mod's inspector
			// iframe) see the page's `engine` through `parent` if they need it.
			if frame.is_none_or(|f| f.is_main() == 0) {
				return;
			}
			let Some(context) = context else { return };
			super::bridge_client::install(context);
			// This engine's marker for the page's scripts, before any of them run.
			let (mut retval, mut exception) = (None, None);
			let marker = modern_engine_script();
			context.eval(Some(&marker.as_str().into()), Some(&"bb://modern_engine.js".into()), 1, Some(&mut retval), Some(&mut exception));
			for (url, script) in SCRIPTS {
				let mut retval = None;
				let mut exception = None;
				context.eval(Some(&script.into()), Some(&url.into()), 1, Some(&mut retval), Some(&mut exception));
				if let Some(exception) = exception {
					eprintln!("bb-ui: {url} failed: {}", CefString::from(&exception.message()));
				}
			}
			#[cfg(target_os = "macos")]
			{
				let (mut retval, mut exception) = (None, None);
				let script = super::system_fonts::page_script();
				let url = "bb://system_fonts.js";
				context.eval(Some(&script.as_str().into()), Some(&url.into()), 1, Some(&mut retval), Some(&mut exception));
				if let Some(exception) = exception {
					eprintln!("bb-ui: {url} failed: {}", CefString::from(&exception.message()));
				}
			}
		}

		fn on_process_message_received(
			&self,
			_browser: Option<&mut Browser>,
			frame: Option<&mut Frame>,
			_source_process: ProcessId,
			message: Option<&mut ProcessMessage>,
		) -> ::std::os::raw::c_int {
			let (Some(frame), Some(message)) = (frame, message) else { return 0 };
			let name = ipc::name(message);
			if name == ipc::TRIM {
				super::memory::trim();
				return 1;
			}
			let id = ipc::int_arg(message, 0);
			let result = match name.as_str() {
				ipc::CALL => call_page(frame, "__bbCall", &ipc::string_arg(message, 1)),
				ipc::EVAL => call_page(frame, "__bbEval", &ipc::string_arg(message, 1)),
				ipc::PARK => park(frame),
				_ => return 0,
			};
			let (ok, text) = match result {
				Some(result) => (1, result),
				None => (0, "the page is not ready".to_string()),
			};
			ipc::send(frame, ProcessId::BROWSER, ipc::REPLY, &[Arg::Int(id), Arg::Int(ok), Arg::Str(&text)]);
			1
		}
	}
}
