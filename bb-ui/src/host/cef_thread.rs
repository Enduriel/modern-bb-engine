//! CEF's own thread: where CEF is initialized and, at the end, shut down.
//!
//! With CEF's multi-threaded message loop (Windows, Linux) the browser runs on CEF's UI
//! thread, and `CefInitialize` only blocks its caller until that thread has started the
//! browser (~135 ms measured, the caller using no CPU meanwhile). On a thread of its
//! own, the game's thread goes on with its start (the archives, the engine, the
//! scripts' preload) instead. CEF must be shut down on the thread that initialized it,
//! so that thread waits until the host is dropped and then shuts CEF down.
//!
//! On macOS CEF's UI thread is the main thread, which runs CEF's work when the game
//! pumps it (`external_message_pump`, [`mac`](super::mac)): CEF is initialized and shut
//! down there, on the caller's thread.

#[cfg(not(target_os = "macos"))]
use std::sync::mpsc;
#[cfg(not(target_os = "macos"))]
use std::thread::JoinHandle;

use cef::args::Args;
use cef::*;

/// Where CEF was initialized (see the module notes); dropping it shuts CEF down there.
pub(super) struct CefThread {
	#[cfg(not(target_os = "macos"))]
	stop: Option<mpsc::Sender<()>>,
	#[cfg(not(target_os = "macos"))]
	thread: Option<JoinHandle<()>>,
	#[cfg(target_os = "macos")]
	_args: Args,
}

impl CefThread {
	/// Start CEF on a thread of its own: `app` and `settings` are made there (CEF's
	/// objects stay on the thread that made them), then `initialize`. `failed` runs
	/// there if CEF couldn't start.
	#[cfg(not(target_os = "macos"))]
	pub fn spawn(
		make: impl FnOnce() -> (App, Settings) + Send + 'static,
		failed: impl FnOnce() + Send + 'static,
	) -> Result<CefThread, String> {
		let (stop, stopped) = mpsc::channel::<()>();
		let thread = std::thread::Builder::new()
			.name("cef-main".into())
			.spawn(move || {
				let args = Args::new();
				let (mut app, settings) = make();
				if initialize(Some(args.as_main_args()), Some(&settings), Some(&mut app), std::ptr::null_mut()) != 1 {
					failed();
					return;
				}
				squirrel_bb::startup::mark("cef started");
				// Until the host is dropped (or gone).
				let _ = stopped.recv();
				shutdown();
			})
			.map_err(|e| format!("CEF's thread: {e}"))?;
		Ok(CefThread { stop: Some(stop), thread: Some(thread) })
	}

	/// Start CEF on this thread, CEF's UI thread on macOS.
	#[cfg(target_os = "macos")]
	pub fn spawn(
		make: impl FnOnce() -> (App, Settings) + Send + 'static,
		_failed: impl FnOnce() + Send + 'static,
	) -> Result<CefThread, String> {
		let args = Args::new();
		let (mut app, settings) = make();
		if initialize(Some(args.as_main_args()), Some(&settings), Some(&mut app), std::ptr::null_mut()) != 1 {
			return Err("CEF failed to initialize (see its cef.log)".into());
		}
		squirrel_bb::startup::mark("cef started");
		Ok(CefThread { _args: args })
	}
}

impl Drop for CefThread {
	fn drop(&mut self) {
		#[cfg(not(target_os = "macos"))]
		{
			drop(self.stop.take());
			if let Some(thread) = self.thread.take() {
				let _ = thread.join();
			}
		}
		#[cfg(target_os = "macos")]
		shutdown();
	}
}
