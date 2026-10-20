//! What CEF needs on macOS that it doesn't elsewhere.
//!
//! - **The framework is loaded at run time** ([`load_framework`]): CEF's C functions are
//!   stubs until `cef_load_library` binds them to `Chromium Embedded Framework.framework`.
//!   In an app bundle it is in `Contents/Frameworks`; a binary run from `target/` (tests,
//!   examples) uses the distribution the build downloaded.
//! - **The application object speaks `CefAppProtocol`** ([`install_application`]): Chromium
//!   asks `NSApp` whether it is inside `sendEvent:` (`CrAppProtocol`), so `NSApp` must be
//!   an `NSApplication` subclass that says so. winit (the window in `bb`) uses whatever
//!   `NSApplication.sharedApplication` returns and only swizzles its `sendEvent:`, so
//!   creating ours first leaves it to winit as is.
//! - **CEF's UI thread is the main thread**: `multi_threaded_message_loop` exists only on
//!   Windows and Linux. The game thread (the main one) runs CEF's work itself
//!   (`external_message_pump`, see [`pump`]) whenever it polls or waits for the page.

use std::cell::Cell;
use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use cef::application_mac::{CefAppProtocol, CrAppControlProtocol, CrAppProtocol};
use objc2::rc::Retained;
use objc2::runtime::{Bool, NSObject, NSObjectProtocol};
use objc2::{ClassType, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSApplication, NSEvent, NSResponder};

/// The framework's binary, inside its bundle.
const FRAMEWORK: &str = "Chromium Embedded Framework.framework/Chromium Embedded Framework";

/// Where the framework is for this executable: in its app bundle (the main program's
/// `Contents/Frameworks`, or for a helper app, the folder its bundle is in), else in the
/// distribution the build found (`cef_dir`).
fn framework_path(cef_dir: &Path) -> Option<PathBuf> {
	let exe = std::env::current_exe().ok()?;
	let dir = exe.parent()?;
	[dir.join("../Frameworks"), dir.join("../../.."), cef_dir.to_path_buf()]
		.into_iter()
		.map(|d| d.join(FRAMEWORK))
		.find(|p| p.is_file())
		.and_then(|p| p.canonicalize().ok())
}

/// The framework's folder (`Chromium Embedded Framework.framework`) for CEF's
/// `framework_dir_path`.
pub(super) fn framework_dir(cef_dir: &Path) -> Option<PathBuf> {
	framework_path(cef_dir).and_then(|p| p.parent().map(Path::to_path_buf))
}

/// Load the framework, once per process, before any other CEF call.
pub(super) fn load_framework(cef_dir: &Path) -> Result<(), String> {
	thread_local! {
		static LOADED: Cell<bool> = const { Cell::new(false) };
	}
	if LOADED.get() {
		return Ok(());
	}
	let path = framework_path(cef_dir).ok_or_else(|| format!("no CEF framework found (looked beside the program and in {})", cef_dir.display()))?;
	let name = CString::new(path.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
	// SAFETY: a valid C string; CEF keeps the library loaded for the process's life.
	if cef::load_library(Some(unsafe { &*name.as_ptr() })) != 1 {
		return Err(format!("could not load {}", path.display()));
	}
	LOADED.set(true);
	Ok(())
}

thread_local! {
	/// Whether `NSApp` is inside `sendEvent:` (`CrAppProtocol`). One application object
	/// per process, used on the main thread only.
	static HANDLING_SEND_EVENT: Cell<bool> = const { Cell::new(false) };
}

define_class!(
	// SAFETY: NSApplication may be subclassed; this one has no ivars and no `Drop`.
	#[unsafe(super(NSApplication, NSResponder, NSObject))]
	#[thread_kind = MainThreadOnly]
	#[name = "BbCefApplication"]
	struct Application;

	unsafe impl NSObjectProtocol for Application {}

	unsafe impl CrAppProtocol for Application {
		#[unsafe(method(isHandlingSendEvent))]
		fn is_handling_send_event(&self) -> Bool {
			Bool::new(HANDLING_SEND_EVENT.get())
		}
	}

	unsafe impl CrAppControlProtocol for Application {
		#[unsafe(method(setHandlingSendEvent:))]
		fn set_handling_send_event(&self, handling: Bool) {
			HANDLING_SEND_EVENT.set(handling.as_bool());
		}
	}

	unsafe impl CefAppProtocol for Application {}

	impl Application {
		#[unsafe(method(sendEvent:))]
		fn send_event(&self, event: &NSEvent) {
			let was = HANDLING_SEND_EVENT.replace(true);
			// SAFETY: NSApplication's `sendEvent:` takes an NSEvent and returns nothing.
			let _: () = unsafe { msg_send![super(self), sendEvent: event] };
			HANDLING_SEND_EVENT.set(was);
		}
	}
);

/// Make `NSApp` an application object that speaks `CefAppProtocol`, before anything
/// else creates one. On the main thread only; a no-op when `NSApp` exists already
/// (then it must be ours, or the page's input misbehaves).
pub(super) fn install_application() -> Result<(), String> {
	let mtm = MainThreadMarker::new().ok_or("CEF must start on the main thread on macOS")?;
	// SAFETY: `sharedApplication` on our subclass creates `NSApp` as an instance of it
	// (or returns the existing one).
	let app: Retained<NSApplication> = unsafe { msg_send![Application::class(), sharedApplication] };
	let _ = mtm;
	if app.isKindOfClass(Application::class()) {
		Ok(())
	} else {
		Err("NSApp was created before the UI host's application object".into())
	}
}

/// Run the CEF work that is due (its UI thread's tasks and callbacks), here on the main
/// thread. Callbacks only pass events to the game, so calling this with no lock held is
/// safe at any point the game waits or polls.
pub(super) fn pump() {
	cef::do_message_loop_work();
}
