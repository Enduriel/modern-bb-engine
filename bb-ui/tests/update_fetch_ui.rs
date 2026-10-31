//! The engine's own requests through Chromium's network stack (`bb_ui::host::fetch`,
//! what the update check asks GitHub with), against a local HTTP server: never the
//! internet. A 200 brings its body and the request carries the check's headers; a 404 is
//! its status; a port where nothing listens fails at once; a server that never answers
//! is given up on at the timeout; an answer larger than the limit is cut off. Skips if
//! there is no Steam install (the page's session is what starts CEF).
//!
//! Run: `cargo test -p bb-ui --features cef --test update_fetch_ui`
//!
//! A plain `main` (`harness = false`): Chromium re-runs this binary for its child
//! processes, and CEF can only start once per process.

use std::cell::RefCell;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::ExitCode;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use bb_engine::harness::STEAM_DATA_DIR;
use bb_ui::host::{Echo, Fetched, HostConfig, UiSession, fetch};

#[path = "support/gpu_page.rs"]
mod gpu_page;

const NAME: &str = "engine_requests_go_through_chromium";

fn ensure(condition: bool, what: impl std::fmt::Display) -> Result<(), String> {
	if condition { Ok(()) } else { Err(what.to_string()) }
}

/// The request's head, as the server read it (up to the blank line).
fn read_head(stream: &mut TcpStream) -> String {
	let mut head = Vec::new();
	let mut byte = [0u8; 1];
	while !head.ends_with(b"\r\n\r\n") {
		match stream.read(&mut byte) {
			Ok(1) => head.push(byte[0]),
			_ => break,
		}
	}
	String::from_utf8_lossy(&head).into_owned()
}

/// A server on a free local port answering one request with `answer` (none: it keeps
/// the connection open, silent, for `hold`). Its address, and the request's head once
/// read.
fn serve_once(answer: Option<(u16, &'static str, String)>, hold: Duration) -> (String, mpsc::Receiver<String>) {
	let listener = TcpListener::bind("127.0.0.1:0").expect("a local port");
	let url = format!("http://127.0.0.1:{}/repos/Enduriel/modern-bb-engine/releases/latest", listener.local_addr().unwrap().port());
	let (heads, head) = mpsc::channel();
	std::thread::spawn(move || {
		let Ok((mut stream, _)) = listener.accept() else { return };
		let _ = heads.send(read_head(&mut stream));
		match answer {
			Some((status, reason, body)) => {
				let response = format!(
					"HTTP/1.1 {status} {reason}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
					body.len()
				);
				let _ = stream.write_all(response.as_bytes());
			}
			None => std::thread::sleep(hold),
		}
	});
	(url, head)
}

/// The answer to `fetch(url)`, the page's session pumped meanwhile (CEF's work runs
/// there on macOS).
fn fetched(session: &UiSession, url: &str, timeout: Duration, max_bytes: usize) -> Result<Result<Fetched, String>, String> {
	let headers = [("User-Agent", "battle-brothers-modern/test"), ("Accept", "application/vnd.github+json")];
	let answer = fetch(url, &headers, timeout, max_bytes);
	let got = RefCell::new(None);
	session.pump_until(timeout + Duration::from_secs(10), |_| {
		if got.borrow().is_none() {
			*got.borrow_mut() = answer.try_recv().ok();
		}
		got.borrow().is_some()
	});
	got.into_inner().ok_or_else(|| format!("no answer from {url}"))
}

fn check(data_dir: &Path) -> Result<(), String> {
	let config = HostConfig { sink: Some(gpu_page::sink()?), ..HostConfig::default() };
	let session = UiSession::start(data_dir, config, Echo::Off)?;
	let json = r#"{"tag_name":"v9.9.9","html_url":"https://github.com/Enduriel/modern-bb-engine/releases/tag/v9.9.9"}"#.to_string();
	let timeout = Duration::from_secs(5);

	// 200: the body as it came, and the headers the check sends.
	let (url, head) = serve_once(Some((200, "OK", json.clone())), Duration::ZERO);
	let answer = fetched(&session, &url, timeout, 1 << 20)?;
	ensure(answer == Ok(Fetched { status: 200, body: json.clone().into_bytes() }), format!("200: {answer:?}"))?;
	let head = head.recv_timeout(Duration::from_secs(1)).map_err(|_| "the server read no request")?;
	let lower = head.to_ascii_lowercase();
	ensure(head.starts_with("GET /repos/Enduriel/modern-bb-engine/releases/latest HTTP/1.1\r\n"), format!("the request: {head}"))?;
	ensure(lower.contains("\r\nuser-agent: battle-brothers-modern/test\r\n"), format!("no User-Agent: {head}"))?;
	ensure(lower.contains("\r\naccept: application/vnd.github+json\r\n"), format!("no Accept: {head}"))?;
	ensure(!lower.contains("\r\ncookie:"), format!("a cookie went out: {head}"))?;

	// 404: an answer, with its status.
	let (url, _head) = serve_once(Some((404, "Not Found", r#"{"message":"Not Found"}"#.into())), Duration::ZERO);
	let answer = fetched(&session, &url, timeout, 1 << 20)?;
	ensure(matches!(&answer, Ok(Fetched { status: 404, .. })), format!("404: {answer:?}"))?;

	// Nothing listening: refused, at once.
	let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
	let start = Instant::now();
	let answer = fetched(&session, &format!("http://127.0.0.1:{port}/"), timeout, 1 << 20)?;
	ensure(
		matches!(&answer, Err(e) if e.contains("ERR_CONNECTION_REFUSED")),
		format!("nothing listening: {answer:?}"),
	)?;
	ensure(start.elapsed() < timeout, format!("refused only after {:?}", start.elapsed()))?;

	// A server that never answers: given up on at the timeout.
	let (url, _head) = serve_once(None, Duration::from_secs(10));
	let start = Instant::now();
	let answer = fetched(&session, &url, Duration::from_secs(1), 1 << 20)?;
	let took = start.elapsed();
	ensure(answer == Err("timed out".to_string()), format!("no answer: {answer:?}"))?;
	ensure(took >= Duration::from_millis(900) && took < Duration::from_secs(4), format!("timed out after {took:?}"))?;

	// More than the limit: cut off.
	let (url, _head) = serve_once(Some((200, "OK", "x".repeat(4096))), Duration::ZERO);
	let answer = fetched(&session, &url, timeout, 1000)?;
	ensure(answer == Err("the answer is too large".to_string()), format!("too large: {answer:?}"))?;
	println!("  200, 404, refused, timed out (after {took:?}), too large");
	Ok(())
}

fn main() -> ExitCode {
	if let Some(code) = bb_ui::host::run_subprocess() {
		return ExitCode::from(code.clamp(0, 255) as u8);
	}
	// A runner's `--list` (cargo-nextest's), answered before a display starts.
	if let Some(code) = bb_engine::harness::list_case(NAME) {
		return code;
	}
	// Chromium paints the page through a display connection: a headless gamescope's.
	bb_ui::host::with_display(run_main)
}

fn run_main() -> ExitCode {
	let data_dir = Path::new(STEAM_DATA_DIR);
	if !data_dir.exists() {
		return bb_engine::harness::skip_case(NAME, format!("{} not present", data_dir.display()));
	}
	match check(data_dir) {
		Ok(()) => {
			println!("test {NAME} ... ok");
			ExitCode::SUCCESS
		}
		Err(error) => {
			println!("test {NAME} ... FAILED\n{error}");
			ExitCode::FAILURE
		}
	}
}
