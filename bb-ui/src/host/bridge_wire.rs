//! The bridge's transport: how the page's blocking calls travel from the renderer
//! process to the game's, outside Chromium's network stack.
//!
//! Each call used to be a synchronous XHR to `coui://ui/__bb__/...`. Every response
//! Chromium sends to the renderer retires message routes of its IPC layer (ipcz), and
//! each side remembers every retired route's id for 5-10 minutes (`NodeLink::
//! deleted_sublinks_`, cleared by time only; crbug 424438875): about 20 ids per call, and
//! the page makes one per call and per park, so both processes grew by tens of MB a
//! battle and kept it. Now a call is a frame on a
//! socket of our own instead.
//!
//! - The game listens on the loopback interface only (`127.0.0.1`, a port the system
//!   picks), and hands the renderer the [`Endpoint`] on its command line.
//! - The first frame on a connection is the endpoint's token, a secret drawn from the
//!   system's randomness; a connection that doesn't open with it is dropped. The token
//!   never reaches the page's scripts: the renderer's native code holds it.
//! - Frames are a little-endian `u32` length, then that many bytes. A request is its
//!   kind, a newline, then its payload; the answer is one frame.
//!
//! Std only, and the same on every system (TCP over loopback).

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};

/// The longest frame taken: a payload past this is a broken or hostile peer.
const MAX_FRAME: u32 = 256 << 20;

/// Where the game listens, and the secret a connection must open with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoint {
	pub port: u16,
	pub token: String,
}

impl Endpoint {
	/// As one command-line value: `port:token`.
	pub fn to_switch(&self) -> String {
		format!("{}:{}", self.port, self.token)
	}

	/// From [`Endpoint::to_switch`]'s form.
	pub fn from_switch(value: &str) -> Option<Endpoint> {
		let (port, token) = value.split_once(':')?;
		Some(Endpoint {
			port: port.parse().ok()?,
			token: token.to_string(),
		})
	}
}

/// 256 bits of the system's randomness, as hex. Std has no random number generator, but
/// each `RandomState` is keyed from the system's (as `HashMap`s are, against flooding).
fn new_token() -> String {
	(0..4)
		.map(|_| {
			let mut hasher = RandomState::new().build_hasher();
			hasher.write_u8(0);
			format!("{:016x}", hasher.finish())
		})
		.collect()
}

/// Whether `a` and `b` are equal, in a time that doesn't tell how much of them is.
fn same_secret(a: &[u8], b: &[u8]) -> bool {
	a.len() == b.len() && a.iter().zip(b).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

pub fn write_frame(stream: &mut impl Write, bytes: &[u8]) -> io::Result<()> {
	let len = u32::try_from(bytes.len())
		.ok()
		.filter(|l| *l <= MAX_FRAME)
		.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "frame too long"))?;
	stream.write_all(&len.to_le_bytes())?;
	stream.write_all(bytes)?;
	stream.flush()
}

pub fn read_frame(stream: &mut impl Read) -> io::Result<Vec<u8>> {
	let mut len = [0; 4];
	stream.read_exact(&mut len)?;
	let len = u32::from_le_bytes(len);
	if len > MAX_FRAME {
		return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too long"));
	}
	let mut bytes = vec![0; len as usize];
	stream.read_exact(&mut bytes)?;
	Ok(bytes)
}

/// The game's end: a loopback listener and its endpoint.
pub struct Listener {
	listener: TcpListener,
	endpoint: Endpoint,
}

impl Listener {
	pub fn bind() -> io::Result<Listener> {
		let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))?;
		let endpoint = Endpoint {
			port: listener.local_addr()?.port(),
			token: new_token(),
		};
		Ok(Listener { listener, endpoint })
	}

	pub fn endpoint(&self) -> &Endpoint {
		&self.endpoint
	}

	/// The next connection that opens with the token. Others are dropped.
	pub fn accept(&self) -> io::Result<TcpStream> {
		loop {
			let (mut stream, _) = self.listener.accept()?;
			stream.set_nodelay(true)?;
			if read_frame(&mut stream).is_ok_and(|t| same_secret(&t, self.endpoint.token.as_bytes())) {
				return Ok(stream);
			}
		}
	}
}

/// The renderer's end: one connection, one call at a time.
pub struct Client {
	stream: TcpStream,
}

impl Client {
	pub fn connect(endpoint: &Endpoint) -> io::Result<Client> {
		let mut stream = TcpStream::connect(SocketAddr::from((Ipv4Addr::LOCALHOST, endpoint.port)))?;
		stream.set_nodelay(true)?;
		write_frame(&mut stream, endpoint.token.as_bytes())?;
		Ok(Client { stream })
	}

	/// Send request `kind` with `payload`; block until the answer.
	pub fn request(&mut self, kind: &str, payload: &str) -> io::Result<Vec<u8>> {
		let mut frame = Vec::with_capacity(kind.len() + 1 + payload.len());
		frame.extend_from_slice(kind.as_bytes());
		frame.push(b'\n');
		frame.extend_from_slice(payload.as_bytes());
		write_frame(&mut self.stream, &frame)?;
		read_frame(&mut self.stream)
	}
}

/// A request frame as `(kind, payload)`.
pub fn split_request(frame: &[u8]) -> Option<(&str, &str)> {
	let text = std::str::from_utf8(frame).ok()?;
	text.split_once('\n')
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn endpoints_survive_the_command_line() {
		let endpoint = Endpoint { port: 4711, token: new_token() };
		assert_eq!(endpoint.token.len(), 64);
		assert_eq!(Endpoint::from_switch(&endpoint.to_switch()), Some(endpoint));
		assert_ne!(new_token(), new_token());
	}

	#[test]
	fn a_call_goes_round_and_strangers_are_dropped() {
		let listener = Listener::bind().unwrap();
		let endpoint = listener.endpoint().clone();
		let server = std::thread::spawn(move || {
			let mut stream = listener.accept().unwrap();
			let frame = read_frame(&mut stream).unwrap();
			let (kind, payload) = split_request(&frame).unwrap();
			write_frame(&mut stream, format!("{kind}|{payload}").as_bytes()).unwrap();
		});
		// A stranger with the wrong token is dropped; the game keeps waiting.
		let wrong = Endpoint { port: endpoint.port, token: "nope".into() };
		let mut stranger = Client::connect(&wrong).unwrap();
		assert!(stranger.request("call", "x").is_err());
		let mut client = Client::connect(&endpoint).unwrap();
		assert_eq!(client.request("park", "{\"a\":\"b\\nc\"}").unwrap(), b"park|{\"a\":\"b\\nc\"}");
		server.join().unwrap();
	}
}
