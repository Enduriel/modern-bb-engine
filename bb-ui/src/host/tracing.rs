//! Chromium's own tracing (CEF's `CefBeginTracing`/`CefEndTracing`), for tools that
//! break a page frame down by stage: every process's trace events, written as a Chrome
//! JSON trace (open it in Perfetto or `chrome://tracing`).
//!
//! Both calls must run on CEF's UI thread: they are posted there and waited for. On
//! Linux and Windows that thread is CEF's own (`multi_threaded_message_loop`); on macOS
//! it is the caller's, which runs CEF's work while it waits (`super::recv`), or the
//! posted task would never run.

use std::sync::mpsc;
use std::time::Duration;

use cef::*;

wrap_completion_callback! {
	struct Started {
		done: mpsc::Sender<()>,
	}

	impl CompletionCallback {
		fn on_complete(&self) {
			let _ = self.done.send(());
		}
	}
}

wrap_end_tracing_callback! {
	struct Written {
		done: mpsc::Sender<()>,
	}

	impl EndTracingCallback {
		fn on_end_tracing_complete(&self, _tracing_file: Option<&CefString>) {
			let _ = self.done.send(());
		}
	}
}

wrap_task! {
	struct Begin {
		categories: String,
		done: mpsc::Sender<()>,
	}

	impl Task {
		fn execute(&self) {
			let mut callback = Started::new(self.done.clone());
			if begin_tracing(Some(&self.categories.as_str().into()), Some(&mut callback)) != 1 {
				eprintln!("bb-ui: tracing did not start");
			}
		}
	}
}

wrap_task! {
	struct End {
		path: String,
		done: mpsc::Sender<()>,
	}

	impl Task {
		fn execute(&self) {
			let mut callback = Written::new(self.done.clone());
			if end_tracing(Some(&self.path.as_str().into()), Some(&mut callback)) != 1 {
				eprintln!("bb-ui: tracing did not end");
			}
		}
	}
}

/// Starts tracing `categories` (Chromium's category filter: `"viz,cc,gpu"`,
/// `"-*,disabled-by-default-viz.quads"`, …) in every process; returns once all have
/// started.
pub fn begin(categories: &str) -> Result<(), String> {
	let (done, started) = mpsc::channel();
	let mut task = Begin::new(categories.to_string(), done);
	if post_task(ThreadId::UI, Some(&mut task)) != 1 {
		return Err("could not post to CEF's UI thread".into());
	}
	super::recv(&started, Duration::from_secs(10)).ok_or_else(|| "tracing did not start".to_string())
}

/// Ends tracing and writes what every process recorded to `path`; returns once written.
pub fn end(path: &std::path::Path) -> Result<(), String> {
	let (done, written) = mpsc::channel();
	let mut task = End::new(path.to_string_lossy().into_owned(), done);
	if post_task(ThreadId::UI, Some(&mut task)) != 1 {
		return Err("could not post to CEF's UI thread".into());
	}
	super::recv(&written, Duration::from_secs(60)).ok_or_else(|| "the trace was not written".to_string())
}
