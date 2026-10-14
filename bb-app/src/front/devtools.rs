//! Where to open DevTools for the page (`bb --devtools PORT`). CEF serves the DevTools
//! frontend itself (`/devtools/inspector.html`), but not Chrome's discovery page: the
//! port's root answers empty. So the page's own inspector URL is printed: its target is
//! found in the port's list (`/json/list`) by the page's URL, on a thread of its own,
//! and printed again whenever it changes. Without one, the hint is `chrome://inspect`.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use bb_ui::json::Json;

/// The page's URL, which names its target.
const PAGE_URL: &str = "coui://ui/main.html";

/// How often the target list is read.
const POLL: Duration = Duration::from_secs(2);

/// How many reads find no page before the `chrome://inspect` hint.
const PATIENCE: u32 = 5;

/// Watch DevTools at `port` and print where to open the page's, now and whenever its
/// target changes.
pub fn announce(port: u16) {
	let watch = move || {
		let mut shown: Option<String> = None;
		let mut misses = 0;
		loop {
			match get(port, "/json/list").ok().as_deref().and_then(body).and_then(page_target) {
				Some(id) if shown.as_deref() != Some(id.as_str()) => {
					eprintln!("DevTools: {} (open in a Chromium-based browser)", inspector_url(port, &id));
					shown = Some(id);
				}
				Some(_) => {}
				None if shown.is_none() => {
					misses += 1;
					if misses == PATIENCE {
						eprintln!("DevTools: in a Chromium-based browser, chrome://inspect, Configure, localhost:{port}");
					}
				}
				None => {}
			}
			std::thread::sleep(POLL);
		}
	};
	if let Err(e) = std::thread::Builder::new().name("devtools".into()).spawn(watch) {
		eprintln!("devtools: {e}");
	}
}

/// The DevTools frontend for target `id`, as the port itself serves it.
fn inspector_url(port: u16, id: &str) -> String {
	format!("http://127.0.0.1:{port}/devtools/inspector.html?ws=127.0.0.1:{port}/devtools/page/{id}")
}

/// The page's target id in a `/json/list` answer.
fn page_target(list: &str) -> Option<String> {
	let list = Json::parse(list).ok()?;
	let page = list
		.as_array()?
		.iter()
		.find(|t| t.get("url").and_then(Json::as_str) == Some(PAGE_URL))?;
	page.get("id").and_then(Json::as_str).map(str::to_string)
}

/// An HTTP response's body.
fn body(response: &str) -> Option<&str> {
	response.split_once("\r\n\r\n").map(|(_, body)| body)
}

/// `GET path` from the local port: the whole response, read up to its
/// `Content-Length` (the server may keep the connection open).
fn get(port: u16, path: &str) -> std::io::Result<String> {
	let mut stream = TcpStream::connect(("127.0.0.1", port))?;
	stream.set_read_timeout(Some(Duration::from_secs(2)))?;
	write!(stream, "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n")?;
	let mut response = Vec::new();
	let mut chunk = [0u8; 16 * 1024];
	loop {
		let n = stream.read(&mut chunk)?;
		if n == 0 {
			break;
		}
		response.extend_from_slice(&chunk[..n]);
		let text = String::from_utf8_lossy(&response);
		if let Some((head, body)) = text.split_once("\r\n\r\n") {
			let length = head
				.lines()
				.find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case("content-length")))
				.and_then(|(_, v)| v.trim().parse::<usize>().ok());
			if length.is_some_and(|length| body.len() >= length) {
				break;
			}
		}
	}
	Ok(String::from_utf8_lossy(&response).into_owned())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn finds_the_pages_target() {
		let list = r#"[ {
   "description": "",
   "devtoolsFrontendUrl": "https://chrome-devtools-frontend.appspot.com/serve_rev/@b859/inspector.html?ws=127.0.0.1:9333/devtools/page/324C",
   "id": "324CE422AFFBEBDCDFC2FE7AAEC1B3F2",
   "title": "coui://ui/main.html",
   "type": "page",
   "url": "coui://ui/main.html",
   "webSocketDebuggerUrl": "ws://127.0.0.1:9333/devtools/page/324CE422AFFBEBDCDFC2FE7AAEC1B3F2"
} ]"#;
		let response = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{list}", list.len());
		let id = body(&response).and_then(page_target);
		assert_eq!(id.as_deref(), Some("324CE422AFFBEBDCDFC2FE7AAEC1B3F2"));
		assert_eq!(
			inspector_url(9333, "324C"),
			"http://127.0.0.1:9333/devtools/inspector.html?ws=127.0.0.1:9333/devtools/page/324C"
		);
		assert_eq!(page_target("[]"), None);
		assert_eq!(page_target(r#"[{"id": "1", "url": "about:blank"}]"#), None);
		assert_eq!(page_target("not json"), None);
	}
}
