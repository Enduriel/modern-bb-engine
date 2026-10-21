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
			let function = match name.as_str() {
				ipc::CALL => "__bbCall",
				ipc::EVAL => "__bbEval",
				ipc::PARK => "__bbPark",
				_ => return 0,
			};
			let id = ipc::int_arg(message, 0);
			let (ok, text) = match call_page(frame, function, &ipc::string_arg(message, 1)) {
				Some(result) => (1, result),
				None => (0, "the page is not ready".to_string()),
			};
			ipc::send(frame, ProcessId::BROWSER, ipc::REPLY, &[Arg::Int(id), Arg::Int(ok), Arg::Str(&text)]);
			1
		}
	}
}
