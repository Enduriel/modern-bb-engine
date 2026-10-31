//! One HTTP GET through Chromium's own network stack (CEF's `CefURLRequest`), for the
//! engine's own requests (the update check): the system's proxy settings and
//! certificates, as the page's, and no HTTP library of our own.
//!
//! The request is made on CEF's UI thread (a posted task, as `CefURLRequest` wants it
//! made on a browser-process thread with a message loop), sends no cookies or stored
//! credentials and keeps nothing in the cache. Its answer comes back on a channel,
//! once: the status and the body, or why there is none. A request still running after
//! its timeout is cancelled, and a body larger than its limit is cut off. On macOS the
//! task runs when the game pumps CEF's work, as all of CEF's there.

use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use cef::sys::cef_urlrequest_flags_t;
use cef::*;

/// A request's answer: the HTTP status and the body as it came.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fetched {
	pub status: i32,
	pub body: Vec<u8>,
}

/// What a request's callbacks share, behind a lock: CEF calls them on its UI thread,
/// and the timeout's task runs there too, but the cef crate's wrapped types are `Send`
/// only through it.
struct State {
	body: Vec<u8>,
	max_bytes: usize,
	/// The answer goes here, once (taken when sent).
	answer: Option<mpsc::Sender<Result<Fetched, String>>>,
	/// Why the request was cancelled, if it was.
	cancelled: Option<&'static str>,
}

impl State {
	fn send(&mut self, result: Result<Fetched, String>) {
		if let Some(answer) = self.answer.take() {
			let _ = answer.send(result);
		}
	}
}

type Shared = Arc<Mutex<State>>;

fn lock(state: &Shared) -> std::sync::MutexGuard<'_, State> {
	state.lock().unwrap_or_else(|e| e.into_inner())
}

wrap_urlrequest_client! {
	struct Client {
		state: Shared,
	}

	impl UrlrequestClient {
		fn on_download_data(&self, request: Option<&mut Urlrequest>, data: *const u8, data_length: usize) {
			let mut state = lock(&self.state);
			if data.is_null() || data_length == 0 || state.cancelled.is_some() {
				return;
			}
			if state.body.len() + data_length > state.max_bytes {
				state.cancelled = Some("the answer is too large");
				drop(state);
				if let Some(request) = request {
					request.cancel();
				}
				return;
			}
			// SAFETY: CEF hands `data_length` readable bytes at `data` for this call.
			let bytes = unsafe { std::slice::from_raw_parts(data, data_length) };
			state.body.extend_from_slice(bytes);
		}

		fn on_request_complete(&self, request: Option<&mut Urlrequest>) {
			let mut state = lock(&self.state);
			let result = match request {
				None => Err("no request".to_string()),
				Some(request) => outcome(request, &mut state),
			};
			state.send(result);
		}
	}
}

/// What a finished `request` came to.
fn outcome(request: &Urlrequest, state: &mut State) -> Result<Fetched, String> {
	if let Some(why) = state.cancelled {
		return Err(why.to_string());
	}
	if request.request_status().get_raw() != UrlrequestStatus::SUCCESS.get_raw() {
		let error = cef::sys::cef_errorcode_t::from(request.request_error());
		return Err(format!("the request failed ({error:?}, {})", error as i32));
	}
	let status = request.response().map_or(0, |response| response.status());
	Ok(Fetched { status, body: std::mem::take(&mut state.body) })
}

wrap_task! {
	struct Start {
		url: String,
		headers: Vec<(String, String)>,
		timeout: Duration,
		state: Shared,
	}

	impl Task {
		fn execute(&self) {
			if let Err(error) = self.start() {
				lock(&self.state).send(Err(error));
			}
		}
	}
}

impl Start {
	fn start(&self) -> Result<(), String> {
		let request = request_create().ok_or("CEF made no request")?;
		request.set_url(Some(&self.url.as_str().into()));
		request.set_method(Some(&"GET".into()));
		for (name, value) in &self.headers {
			request.set_header_by_name(Some(&name.as_str().into()), Some(&value.as_str().into()), 1);
		}
		// Nothing from the cache nor into it; no cookies (stored credentials are only sent
		// with UR_FLAG_ALLOW_STORED_CREDENTIALS).
		let flags = cef_urlrequest_flags_t::UR_FLAG_SKIP_CACHE.0 | cef_urlrequest_flags_t::UR_FLAG_DISABLE_CACHE.0;
		request.set_flags(flags as i32);
		let mut request = request;
		let mut client = Client::new(self.state.clone());
		let running = urlrequest_create(Some(&mut request), Some(&mut client), None).ok_or("CEF did not start the request")?;
		let mut cancel = Cancel::new(running, self.state.clone());
		let ms = i64::try_from(self.timeout.as_millis()).unwrap_or(i64::MAX);
		if post_delayed_task(ThreadId::UI, Some(&mut cancel), ms) != 1 {
			return Err("could not post the request's timeout".into());
		}
		Ok(())
	}
}

wrap_task! {
	struct Cancel {
		request: Urlrequest,
		state: Shared,
	}

	impl Task {
		fn execute(&self) {
			let mut state = lock(&self.state);
			if state.answer.is_none() {
				return;
			}
			state.cancelled = Some("timed out");
			drop(state);
			// Completes the request (cancelled), which sends the answer.
			self.request.cancel();
		}
	}
}

/// GET `url` with `headers` through CEF (which must be running: a [`UiSession`] open),
/// waiting at most `timeout` for the whole answer and taking at most `max_bytes` of it.
/// The answer arrives on the returned channel, once, whatever happens.
///
/// [`UiSession`]: super::UiSession
pub fn fetch(url: &str, headers: &[(&str, &str)], timeout: Duration, max_bytes: usize) -> mpsc::Receiver<Result<Fetched, String>> {
	let (answer, answered) = mpsc::channel();
	let state = Arc::new(Mutex::new(State { body: Vec::new(), max_bytes, answer: Some(answer), cancelled: None }));
	let headers = headers.iter().map(|(n, v)| (n.to_string(), v.to_string())).collect();
	let mut task = Start::new(url.to_string(), headers, timeout, state.clone());
	if post_task(ThreadId::UI, Some(&mut task)) != 1 {
		lock(&state).send(Err("could not post to CEF's UI thread".into()));
	}
	answered
}
