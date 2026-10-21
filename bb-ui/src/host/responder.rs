//! How the game thread answers what the page waits on: a bridge call (over the
//! bridge's socket, [`bridge_server`](super::bridge_server)) or a procedural image (a
//! `coui://` load, [`scheme`](super::scheme)).

use std::sync::OnceLock;

/// What the page reads back.
pub(super) struct Body {
	pub mime: &'static str,
	pub data: Vec<u8>,
	pub offset: usize,
}

impl Body {
	pub fn new(mime: &'static str, data: Vec<u8>) -> Body {
		Body { mime, data, offset: 0 }
	}
}

/// A 1×1 transparent PNG.
pub(super) fn blank_image() -> &'static [u8] {
	static IMAGE: OnceLock<Vec<u8>> = OnceLock::new();
	IMAGE.get_or_init(|| {
		let mut png = Vec::new();
		crate::png::write_rgba(&mut png, 1, 1, &[0, 0, 0, 0]).expect("writing to a Vec");
		png
	})
}

/// What an unanswered bridge call answers.
pub(super) fn done() -> Body {
	Body::new("application/json", br#"{"done":true}"#.to_vec())
}

/// What an unanswered procedural image answers.
pub(super) fn blank() -> Body {
	Body::new("image/png", blank_image().to_vec())
}

/// Answers one thing the page waits on. Answering (or dropping it unanswered, which
/// answers its fallback: "done", or the blank image) lets the page continue.
pub struct Responder {
	/// Takes the answer; `None` fails the load (an image's error, an XHR's status 0).
	deliver: Option<Box<dyn FnOnce(Option<Body>) + Send>>,
	/// What it answers if dropped unanswered.
	fallback: fn() -> Body,
}

impl Responder {
	pub(super) fn new(fallback: fn() -> Body, deliver: impl FnOnce(Option<Body>) + Send + 'static) -> Responder {
		Responder { deliver: Some(Box::new(deliver)), fallback }
	}

	/// Answer with `json`.
	pub fn respond(mut self, json: String) {
		self.finish(Some(Body::new("application/json", json.into_bytes())));
	}

	/// Answer with an image: `png`, or the transparent placeholder.
	pub fn respond_image(mut self, png: Option<Vec<u8>>) {
		self.finish(Some(Body::new("image/png", png.unwrap_or_else(|| blank_image().to_vec()))));
	}

	/// Fail the load, as a missing file fails: an image's `error` event, an XHR's
	/// status 0.
	pub fn fail(mut self) {
		self.finish(None);
	}

	fn finish(&mut self, body: Option<Body>) {
		if let Some(deliver) = self.deliver.take() {
			deliver(body);
		}
	}
}

impl Drop for Responder {
	fn drop(&mut self) {
		// Never leave the page waiting.
		let fallback = (self.fallback)();
		self.finish(Some(fallback));
	}
}

impl std::fmt::Debug for Responder {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str("Responder")
	}
}
