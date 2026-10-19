//! The game's end of the bridge: the page's blocking calls arrive on the bridge's
//! socket ([`bridge_wire`](super::bridge_wire)) and go to the game thread as
//! [`Event::Request`]s; each connection waits for its [`Responder`]'s answer and writes
//! it back. One renderer, one connection; a new renderer (after a crash) connects anew.

use std::net::TcpStream;
use std::sync::{Arc, mpsc};

use super::bridge_wire::{Endpoint, Listener, read_frame, split_request, write_frame};
use super::client::{Event, Shared};
use super::responder::{Responder, done};

/// Which bridge call the page made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestKind {
	/// `engine.call(...)` from inside one of the game's calls into the page: run a
	/// handler in the game.
	Call,
	/// `engine.call(...)` from one of the page's own tasks (an animation frame, a
	/// timer, input): the same, but the game wasn't calling the page.
	Task,
	/// The result of a call the game made into the page while the page was waiting.
	Resume,
	/// The page parks: it waits until the game lets it go on (`UiHost::park`).
	Park,
}

impl RequestKind {
	fn parse(kind: &str) -> Option<RequestKind> {
		Some(match kind {
			"call" => RequestKind::Call,
			"task" => RequestKind::Task,
			"resume" => RequestKind::Resume,
			"park" => RequestKind::Park,
			_ => return None,
		})
	}
}

/// Listen for the page's renderer. Returns the endpoint to hand it.
pub(super) fn start(listener: Listener, shared: Arc<Shared>) -> Endpoint {
	let endpoint = listener.endpoint().clone();
	let spawned = std::thread::Builder::new().name("bb-bridge".into()).spawn(move || {
		while let Ok(stream) = listener.accept() {
			let shared = shared.clone();
			let _ = std::thread::Builder::new()
				.name("bb-bridge-page".into())
				.spawn(move || serve(stream, &shared));
		}
	});
	if let Err(e) = spawned {
		eprintln!("bb-ui: the bridge could not start: {e}");
	}
	endpoint
}

/// One renderer's calls, in turn, until it goes away.
fn serve(mut stream: TcpStream, shared: &Shared) {
	while let Ok(frame) = read_frame(&mut stream) {
		let request = split_request(&frame).and_then(|(k, p)| Some((RequestKind::parse(k)?, p.to_string())));
		let Some((kind, body)) = request else { return };
		let (tx, rx) = mpsc::sync_channel(1);
		let responder = Responder::new(done, move |answer| {
			let _ = tx.send(answer.data);
		});
		shared.send(Event::Request { kind, body, responder });
		// The game answers, or drops the responder (which answers "done"); if the
		// game's side is gone altogether, so is the page's reason to wait.
		let Ok(answer) = rx.recv() else { return };
		if write_frame(&mut stream, &answer).is_err() {
			return;
		}
	}
}
