//! Wayland's clipboard through the game's own connection to the compositor
//! (`smithay-clipboard`), so no clipboard program (wl-clipboard) is needed.
//!
//! The compositor lets only a window with the keyboard's focus take or read the
//! selection, with the serial of an input event it got: the game's window. So the
//! clipboard works once the window has had a key, a modifier or a click (a copy's
//! Ctrl+C, a paste's Ctrl+V, a mod's Export button).
//!
//! `smithay-clipboard` serves the selection from a thread of its own; its `load` waits
//! for the selection's owner without a limit. It is driven from another thread here
//! ([`Wayland`]), which a read waits for only up to [`TIMEOUT`]: a selection's owner
//! that never answers can't stop the page's copy and paste.

use std::io;
use std::sync::mpsc::{self, Sender};
use std::time::Duration;

use winit::event_loop::OwnedDisplayHandle;
use winit::raw_window_handle::{HasDisplayHandle, RawDisplayHandle};

/// How long a read may take (as long as a clipboard program may, `command::TIMEOUT`).
const TIMEOUT: Duration = Duration::from_secs(1);

/// The text a dead `smithay-clipboard` answers a load with: its thread stopped, as it
/// does at once when the compositor has neither clipboard (gamescope's has none).
const DEAD: &str = "clipboard is dead.";
/// ... and a live one when the compositor has only the primary selection.
const UNSUPPORTED: &str = "requested selection is not supported";

/// Wayland's clipboard, served on the game's own connection.
pub(super) struct Wayland {
	requests: Sender<Request>,
}

/// What the clipboard's thread is asked.
enum Request {
	Read(Sender<Result<String, ()>>),
	Write(String),
}

/// A clipboard the thread drives: `smithay-clipboard`'s, or a test's.
trait Selection {
	fn load(&self) -> io::Result<String>;
	fn store(&self, text: String);
}

/// `smithay-clipboard`'s clipboard, and the connection it is on.
struct Served {
	// Dropped before the connection it uses.
	clipboard: smithay_clipboard::Clipboard,
	_display: OwnedDisplayHandle,
}

impl Selection for Served {
	fn load(&self) -> io::Result<String> {
		self.clipboard.load()
	}

	fn store(&self, text: String) {
		self.clipboard.store(text);
	}
}

impl Wayland {
	/// The clipboard on the connection of `display` if it is a Wayland display whose
	/// compositor has a clipboard; `None` otherwise (an X11 display, gamescope).
	pub(super) fn start(display: &OwnedDisplayHandle) -> Option<Wayland> {
		wayland_display(display.display_handle().ok()?.as_raw())?;
		let display = display.clone();
		Wayland::serve(move || {
			let pointer = display.display_handle().ok().and_then(|d| wayland_display(d.as_raw()))?;
			// SAFETY: `pointer` is winit's `wl_display`, whose connection `display` holds
			// open for as long as the clipboard lives (`Served` drops it last).
			let clipboard = unsafe { smithay_clipboard::Clipboard::new(pointer.as_ptr()) };
			Some(Served { clipboard, _display: display })
		})
	}

	/// Start the thread that drives the clipboard `make` makes, once it says the
	/// clipboard is usable ([`usable`]).
	fn serve<S: Selection, F: FnOnce() -> Option<S> + Send + 'static>(make: F) -> Option<Wayland> {
		let (requests, incoming) = mpsc::channel::<Request>();
		let (ready, started) = mpsc::channel();
		let thread = std::thread::Builder::new().name("bb-clipboard".into()).spawn(move || {
			let Some(selection) = make() else {
				let _ = ready.send(false);
				return;
			};
			// Its first answer says whether it serves the compositor's clipboard.
			let works = usable(&selection.load());
			let _ = ready.send(works);
			if !works {
				return;
			}
			// Until the clipboard ([`Wayland`]) is dropped.
			for request in incoming {
				match request {
					Request::Read(reply) => {
						let _ = reply.send(selection.load().map_err(|_| ()));
					}
					Request::Write(text) => selection.store(text),
				}
			}
		});
		thread.ok()?;
		started.recv_timeout(TIMEOUT).unwrap_or(false).then_some(Wayland { requests })
	}

	pub(super) fn name(&self) -> &'static str {
		"Wayland"
	}

	/// The clipboard's text; `Err` if it can't be read (no focus or input on the game's
	/// window yet, nothing on the clipboard, no text on it, no answer in time).
	pub(super) fn read(&self) -> Result<Option<String>, ()> {
		let (reply, answer) = mpsc::channel();
		self.requests.send(Request::Read(reply)).map_err(|_| ())?;
		let text = answer.recv_timeout(TIMEOUT).map_err(|_| ())??;
		Ok(Some(text).filter(|t| !t.is_empty()))
	}

	/// Put `text` on the clipboard. The compositor doesn't say whether it took it (it
	/// doesn't without the window's focus).
	pub(super) fn write(&self, text: &str) -> Result<(), ()> {
		self.requests.send(Request::Write(text.to_string())).map_err(|_| ())
	}
}

/// The `wl_display` behind `display`, if it is Wayland's.
fn wayland_display(display: RawDisplayHandle) -> Option<std::ptr::NonNull<std::ffi::c_void>> {
	match display {
		RawDisplayHandle::Wayland(handle) => Some(handle.display),
		_ => None,
	}
}

/// Whether a clipboard's first load says it serves the compositor's clipboard: any
/// answer but a dead thread's or a missing clipboard's (with nothing copied yet and no
/// input on the window, a working one answers with an error too).
fn usable(first: &io::Result<String>) -> bool {
	match first {
		Ok(_) => true,
		Err(e) => {
			let e = e.to_string();
			e != DEAD && e != UNSUPPORTED
		}
	}
}

#[cfg(test)]
mod tests {
	use std::io;
	use std::ptr::NonNull;
	use std::sync::{Arc, Mutex};
	use std::time::{Duration, Instant};

	use winit::raw_window_handle::{RawDisplayHandle, WaylandDisplayHandle, XlibDisplayHandle};

	use super::{Selection, TIMEOUT, Wayland, usable, wayland_display};

	/// A clipboard in memory, whose loads can be made to hang.
	#[derive(Clone, Default)]
	struct Fake {
		text: Arc<Mutex<Option<String>>>,
		hang: Arc<Mutex<Option<Duration>>>,
	}

	impl Selection for Fake {
		fn load(&self) -> io::Result<String> {
			if let Some(hang) = *self.hang.lock().unwrap() {
				std::thread::sleep(hang);
			}
			self.text.lock().unwrap().clone().ok_or_else(|| io::Error::other("selection is empty"))
		}

		fn store(&self, text: String) {
			*self.text.lock().unwrap() = Some(text);
		}
	}

	#[test]
	fn only_a_wayland_display_is_served() {
		let pointer = NonNull::<u8>::dangling().cast();
		let wayland = RawDisplayHandle::Wayland(WaylandDisplayHandle::new(pointer));
		assert_eq!(wayland_display(wayland), Some(pointer));
		let x11 = RawDisplayHandle::Xlib(XlibDisplayHandle::new(None, 0));
		assert_eq!(wayland_display(x11), None);
	}

	#[test]
	fn a_dead_or_primary_only_clipboard_is_not_used() {
		assert!(usable(&Ok("text".into())));
		assert!(usable(&Err(io::Error::other("no events received on any seat"))));
		assert!(usable(&Err(io::Error::other("selection is empty"))));
		assert!(!usable(&Err(io::Error::other("clipboard is dead."))));
		assert!(!usable(&Err(io::Error::other("requested selection is not supported"))));
	}

	#[test]
	fn the_clipboard_is_not_used_if_it_cannot_start() {
		assert!(Wayland::serve(|| None::<Fake>).is_none());
		struct Dead;
		impl Selection for Dead {
			fn load(&self) -> io::Result<String> {
				Err(io::Error::other("clipboard is dead."))
			}
			fn store(&self, _: String) {}
		}
		assert!(Wayland::serve(|| Some(Dead)).is_none());
	}

	#[test]
	fn reads_and_writes_go_through_in_order() {
		let fake = Fake::default();
		let clipboard = Wayland::serve({
			let fake = fake.clone();
			move || Some(fake)
		})
		.expect("served");
		assert_eq!(clipboard.read(), Err(()), "nothing copied yet");
		clipboard.write("first").unwrap();
		clipboard.write("Ünïcode\nsecond").unwrap();
		assert_eq!(clipboard.read(), Ok(Some("Ünïcode\nsecond".to_string())));
		clipboard.write("").unwrap();
		assert_eq!(clipboard.read(), Ok(None), "an empty copy reads as none");
	}

	#[test]
	fn a_read_that_hangs_gives_up() {
		let fake = Fake::default();
		let clipboard = Wayland::serve({
			let fake = fake.clone();
			move || Some(fake)
		})
		.expect("served");
		clipboard.write("text").unwrap();
		*fake.hang.lock().unwrap() = Some(TIMEOUT * 2);
		let since = Instant::now();
		assert_eq!(clipboard.read(), Err(()));
		let waited = since.elapsed();
		assert!(waited >= TIMEOUT && waited < TIMEOUT * 2, "waited {waited:?}");
	}

	/// Copies to and pastes from a real compositor's clipboard through a window of its
	/// own, so only when asked (ignored; `--include-ignored` and `BB_CLIPBOARD_TEST=1`), in a compositor of its own
	/// (`WAYLAND_DISPLAY`; never the player's: it would replace their clipboard). It
	/// needs one with a clipboard and that focuses a new window, as a headless KWin
	/// does: `kwin_wayland --virtual --exit-with-session <script>` under
	/// `dbus-run-session`, with its own `XDG_RUNTIME_DIR` (a short path: the socket's
	/// must fit 108 bytes), `HOME` and `XDG_CONFIG_HOME`, so it can't reach the player's
	/// session or rewrite their KWin settings. `BB_CLIPBOARD_WAYLAND=none` says the compositor
	/// has no clipboard, as gamescope's: the window's clipboard then falls back to the
	/// programs (`command`).
	#[cfg(target_os = "linux")]
	#[test]
	#[ignore = "opt-in: BB_CLIPBOARD_TEST=1, inside a Wayland compositor of its own"]
	fn text_goes_round_a_compositors_clipboard() {
		use winit::event_loop::EventLoop;
		use winit::platform::pump_events::EventLoopExtPumpEvents;
		use winit::platform::wayland::EventLoopBuilderExtWayland;

		// Run by name, it still never touches the player's session unasked.
		assert!(
			std::env::var_os("BB_CLIPBOARD_TEST").is_some_and(|v| v == "1") && std::env::var_os("WAYLAND_DISPLAY").is_some(),
			"set BB_CLIPBOARD_TEST=1 inside a Wayland compositor of its own"
		);
		let mut event_loop = EventLoop::builder().with_wayland().with_any_thread(true).build().expect("a Wayland event loop");
		let display = event_loop.owned_display_handle();
		if std::env::var_os("BB_CLIPBOARD_WAYLAND").is_some_and(|v| v == "none") {
			assert!(Wayland::start(&display).is_none(), "the compositor has no clipboard");
			let system = crate::clipboard::SystemClipboard::for_window(&display);
			eprintln!("falls back to: {:?}", system.system_name());
			assert_ne!(system.system_name(), Some("Wayland"));
			return;
		}
		let clipboard = Wayland::start(&display).expect("the compositor's clipboard");
		// A window shown (a frame presented) for the compositor to focus.
		#[allow(deprecated)]
		let window = Arc::new(event_loop.create_window(winit::window::Window::default_attributes().with_title("bb clipboard test")).expect("a window"));
		let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle_from_env(Box::new(display.clone())));
		let surface = instance.create_surface(window.clone()).expect("a surface");
		let gpu = bb_render::Gpu::for_surface(&instance, &surface).expect("a GPU");
		let size = window.inner_size();
		let capabilities = gpu.surface_capabilities(&surface).expect("surface capabilities");
		let config = wgpu::SurfaceConfiguration {
			usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
			format: capabilities.formats[0],
			width: size.width.max(1),
			height: size.height.max(1),
			present_mode: wgpu::PresentMode::Fifo,
			desired_maximum_frame_latency: 2,
			alpha_mode: capabilities.alpha_modes[0],
			view_formats: Vec::new(),
			color_space: wgpu::SurfaceColorSpace::Auto,
		};
		surface.configure(&gpu.device, &config);
		let mut present = || {
			let (wgpu::CurrentSurfaceTexture::Success(frame) | wgpu::CurrentSurfaceTexture::Suboptimal(frame)) = surface.get_current_texture() else {
				return;
			};
			let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
			let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
			encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
				label: Some("clipboard test"),
				color_attachments: &[Some(wgpu::RenderPassColorAttachment {
					view: &view,
					depth_slice: None,
					resolve_target: None,
					ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
				})],
				depth_stencil_attachment: None,
				timestamp_writes: None,
				occlusion_query_set: None,
				multiview_mask: None,
			});
			gpu.queue.submit([encoder.finish()]);
			gpu.queue.present(frame);
		};
		let pump = |event_loop: &mut EventLoop<()>| {
			let _ = event_loop.pump_app_events(Some(Duration::from_millis(20)), &mut Ignore);
		};
		let round_trip = |text: &str, event_loop: &mut EventLoop<()>, present: &mut dyn FnMut()| {
			let deadline = Instant::now() + Duration::from_secs(10);
			loop {
				present();
				pump(event_loop);
				clipboard.write(text).unwrap();
				pump(event_loop);
				if clipboard.read() == Ok(Some(text.to_string())) {
					return;
				}
				assert!(Instant::now() < deadline, "the compositor never gave the copy back");
			}
		};
		round_trip("Ünïcode \"build\" code\nwith 2 lines | $(id)", &mut event_loop, &mut present);
		let long = "x".repeat(300_000);
		round_trip(&long, &mut event_loop, &mut present);
		drop(clipboard);
		// As `bb` does at quit (gamescope's WSI frees the window's surface itself).
		std::mem::forget(surface);
		std::mem::forget(window);

		struct Ignore;
		impl winit::application::ApplicationHandler for Ignore {
			fn resumed(&mut self, _: &winit::event_loop::ActiveEventLoop) {}
			fn window_event(&mut self, _: &winit::event_loop::ActiveEventLoop, _: winit::window::WindowId, _: winit::event::WindowEvent) {}
		}
	}
}
