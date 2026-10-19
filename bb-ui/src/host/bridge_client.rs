//! The renderer's end of the bridge: `__bbRequest(kind, payload)`, a native function
//! that sends one blocking call to the game over the bridge's socket
//! ([`bridge_wire`](super::bridge_wire)) and returns the answer's text, as the
//! synchronous XHR it replaces did. It throws when the game can't be reached.
//!
//! The endpoint comes from the renderer's command line ([`SWITCH`]), put there by the
//! game's process; the token stays in this code, out of the page's reach. `bootstrap.js`
//! takes the function off `window` before any page script runs.

use std::sync::Mutex;

use cef::*;

use super::bridge_wire::{Client, Endpoint};

/// The renderer's switch carrying the endpoint (`port:token`).
pub(super) const SWITCH: &str = "bb-bridge";

/// The page's name for the function.
const FUNCTION: &str = "__bbRequest";

/// The renderer's connection to the game, made at the first call.
static CLIENT: Mutex<Option<Client>> = Mutex::new(None);

fn endpoint() -> Option<Endpoint> {
	let command_line = command_line_get_global()?;
	let value = CefString::from(&command_line.switch_value(Some(&SWITCH.into()))).to_string();
	Endpoint::from_switch(&value)
}

/// Send one call; its answer's text.
fn request(kind: &str, payload: &str) -> Result<String, String> {
	let mut client = CLIENT.lock().unwrap_or_else(|e| e.into_inner());
	if client.is_none() {
		let endpoint = endpoint().ok_or("the bridge's endpoint is missing")?;
		*client = Some(Client::connect(&endpoint).map_err(|e| format!("the game is not reachable: {e}"))?);
	}
	let answer = client.as_mut().map(|c| c.request(kind, payload));
	match answer {
		Some(Ok(bytes)) => String::from_utf8(bytes).map_err(|_| "the game's answer is not text".into()),
		Some(Err(e)) => {
			// Reconnect at the next call (the game may have restarted the page).
			*client = None;
			Err(format!("the game did not answer: {e}"))
		}
		None => Err("no connection".into()),
	}
}

fn string_arg(arguments: Option<&[Option<V8Value>]>, index: usize) -> Option<String> {
	let value = arguments?.get(index)?.as_ref()?;
	(value.is_string() != 0).then(|| CefString::from(&value.string_value()).to_string())
}

wrap_v8_handler! {
	struct BridgeFunction {}

	impl V8Handler {
		fn execute(
			&self,
			_name: Option<&CefString>,
			_object: Option<&mut V8Value>,
			arguments: Option<&[Option<V8Value>]>,
			retval: Option<&mut Option<V8Value>>,
			exception: Option<&mut CefString>,
		) -> ::std::os::raw::c_int {
			let result = match (string_arg(arguments, 0), string_arg(arguments, 1)) {
				(Some(kind), Some(payload)) => request(&kind, &payload),
				_ => Err("__bbRequest(kind, payload) takes two strings".into()),
			};
			match result {
				Ok(text) => {
					if let Some(retval) = retval {
						*retval = v8_value_create_string(Some(&text.as_str().into()));
					}
				}
				Err(error) => {
					if let Some(exception) = exception {
						*exception = error.as_str().into();
					}
				}
			}
			1
		}
	}
}

/// Put `__bbRequest` on `context`'s `window`, for `bootstrap.js` to take.
pub(super) fn install(context: &V8Context) {
	let Some(global) = context.global() else { return };
	let mut handler = BridgeFunction::new();
	let Some(mut function) = v8_value_create_function(Some(&FUNCTION.into()), Some(&mut handler)) else {
		return;
	};
	global.set_value_bykey(Some(&FUNCTION.into()), Some(&mut function), Default::default());
}
