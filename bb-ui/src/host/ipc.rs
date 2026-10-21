//! The process messages from the browser process (game) to the renderer (page), and
//! back. (The page's own calls into the game go through the bridge requests in
//! [`scheme`](super::scheme): a blocked renderer can't receive process messages.)
//!
//! Browser → renderer:
//! - [`CALL`] `(id, entry)`: run an SQ→JS entry now and reply (`__bbCall`);
//! - [`EVAL`] `(id, code)`: evaluate test code and reply (`__bbEval`);
//! - [`PARK`] `(id)`: park (`__bbPark`, a step at a time: `renderer::park`), replying
//!   once the game lets the page go on, or at once if the page has no bridge yet;
//! - [`TRIM`] `()`: give the C allocator's free memory back to the system
//!   ([`memory`](super::memory)); no reply.
//!
//! Renderer → browser:
//! - [`REPLY`] `(id, ok, text)`: the answer to a `CALL`, `EVAL` or `PARK`: its JSON
//!   result if `ok` is 1, else why it couldn't run.

use cef::*;

pub const CALL: &str = "bb.call";
pub const EVAL: &str = "bb.eval";
pub const PARK: &str = "bb.park";
pub const TRIM: &str = "bb.trim";
pub const REPLY: &str = "bb.reply";

/// An argument of a process message.
pub enum Arg<'a> {
	Int(i32),
	Str(&'a str),
}

/// Send message `name` with `args` to `target` through `frame`.
pub fn send(frame: &Frame, target: ProcessId, name: &str, args: &[Arg]) -> bool {
	let Some(mut message) = process_message_create(Some(&name.into())) else {
		return false;
	};
	let Some(list) = message.argument_list() else {
		return false;
	};
	for (i, arg) in args.iter().enumerate() {
		match arg {
			Arg::Int(n) => list.set_int(i, *n),
			Arg::Str(s) => list.set_string(i, Some(&(*s).into())),
		};
	}
	frame.send_process_message(target, Some(&mut message));
	true
}

/// The name of a received message.
pub fn name(message: &ProcessMessage) -> String {
	CefString::from(&message.name()).to_string()
}

/// String argument `index` of a received message (empty if missing).
pub fn string_arg(message: &ProcessMessage, index: usize) -> String {
	message
		.argument_list()
		.map(|list| CefString::from(&list.string(index)).to_string())
		.unwrap_or_default()
}

/// Integer argument `index` of a received message (0 if missing).
pub fn int_arg(message: &ProcessMessage, index: usize) -> i32 {
	message.argument_list().map_or(0, |list| list.int(index))
}
