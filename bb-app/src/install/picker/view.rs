//! The not-found window: a window of its own with the page drawn in it, as the game
//! draws its UI page (bb-render's [`UiLayer`] over a frame, presented on the window's
//! surface), and the player's mouse and keys passed to the page.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use bb_render::{Gpu, Presenter, RenderTarget, UiLayer, UiPage, save_png};
use bb_ui::coui::UiFiles;
use bb_ui::host::{HostConfig, Modifiers, MouseButton, PageFrames, UiHost};
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::ModifiersState;
use winit::window::Window;

use crate::requirements;

/// The window's size at first, in the system's points.
const SIZE: (f64, f64) = (880.0, 700.0);

/// How long the page may take to open.
const OPEN_TIMEOUT: Duration = Duration::from_secs(30);

pub struct View {
	/// `None` once taken down (see [`Drop`]).
	window: Option<Arc<Window>>,
	/// The game's icon on the window, where it takes more than its attributes.
	_icon: crate::icon::Shown,
	surface: Option<wgpu::Surface<'static>>,
	config: wgpu::SurfaceConfiguration,
	gpu: Gpu,
	presenter: Presenter,
	target: RenderTarget,
	layer: UiLayer,
	/// `None` once taken down.
	host: Option<UiHost>,
	/// The paints drawn so far: a new one is drawn at the next redraw.
	drawn_paints: u64,
	/// Where the pointer is, in the page's pixels.
	pointer: (i32, i32),
	modifiers: Modifiers,
}

impl View {
	/// Open the window with the page at `url` in it. Fails as the game's start does on a
	/// machine that can't run it ([`requirements`]).
	pub fn open(event_loop: &ActiveEventLoop, instance: &wgpu::Instance, url: String, cef_log: Option<std::path::PathBuf>) -> Result<View, String> {
		let attributes = crate::icon::with_icon(Window::default_attributes())
			.with_title("Battle Brothers")
			.with_inner_size(LogicalSize::new(SIZE.0, SIZE.1))
			.with_min_inner_size(LogicalSize::new(480.0, 420.0));
		let window = Arc::new(event_loop.create_window(attributes).map_err(|e| e.to_string())?);
		let icon = crate::icon::show(&window, event_loop);
		let surface = instance.create_surface(window.clone()).map_err(|e| format!("no surface for the window: {e}"))?;
		let gpu = Gpu::for_surface_preferring(instance, &surface, UiPage::FEATURES)
			.map_err(|e| requirements::unsupported(format!("no GPU could draw to the game's window ({e})")))?;
		requirements::check_gpu(&gpu)?;
		let capabilities = gpu.surface_capabilities(&surface).ok_or("no surface capabilities")?;
		let format = capabilities
			.formats
			.iter()
			.copied()
			.find(|f| !f.is_srgb())
			.or(capabilities.formats.first().copied())
			.ok_or("the surface offers no format")?;
		let size = window.inner_size();
		let config = wgpu::SurfaceConfiguration {
			usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
			format,
			width: size.width.max(1),
			height: size.height.max(1),
			present_mode: wgpu::PresentMode::Fifo,
			desired_maximum_frame_latency: 2,
			alpha_mode: if capabilities.alpha_modes.contains(&wgpu::CompositeAlphaMode::Opaque) {
				wgpu::CompositeAlphaMode::Opaque
			} else {
				capabilities.alpha_modes[0]
			},
			view_formats: Vec::new(),
			color_space: wgpu::SurfaceColorSpace::Auto,
		};
		surface.configure(&gpu.device, &config);
		let page = UiPage::new(&gpu)?;
		let scale = window.scale_factor();
		let (width, height) = css_size(size, scale);
		let host_config = HostConfig {
			url,
			width: width as u32,
			height: height as u32,
			// Chromium paints the page when it changes, on its own clock.
			frames: PageFrames::Clock(60),
			cef_log,
			// Paste reaches the folder's box from the player's other programs.
			clipboard: Arc::new(crate::clipboard::SystemClipboard::for_window(&event_loop.owned_display_handle())),
			sink: Some(crate::front::gpu_page::sink(page.clone())),
			// The folder dialog (Chromium's, on Windows) opens attached to this window.
			parent_window: win32_handle(&window),
			..HostConfig::default()
		};
		// No archives: the page loads nothing from the game (there is none yet).
		let files = UiFiles::open(&[], None).map_err(|e| e.to_string())?;
		let host = UiHost::start(Arc::new(files), host_config).map_err(requirements::page_error)?;
		host.wait_open(OPEN_TIMEOUT).map_err(requirements::page_error)?;
		host.resize(width, height, scale);
		Ok(View {
			target: RenderTarget::new(&gpu, config.width, config.height),
			presenter: Presenter::new(&gpu, format),
			layer: UiLayer::new(&gpu, page),
			window: Some(window),
			_icon: icon,
			surface: Some(surface),
			config,
			gpu,
			host: Some(host),
			drawn_paints: 0,
			pointer: (0, 0),
			modifiers: Modifiers::default(),
		})
	}

	pub fn host(&self) -> &UiHost {
		self.host.as_ref().expect("the page, until the view is taken down")
	}

	pub fn window(&self) -> &Window {
		self.window.as_ref().expect("the window, until the view is taken down")
	}

	/// Whether the page has painted at all.
	pub fn painted(&self) -> bool {
		self.host().paints() > 0
	}

	/// Whether the page painted since it was last drawn.
	pub fn changed(&self) -> bool {
		self.host().paints() != self.drawn_paints
	}

	/// The window is `size` now: the surface, the frame and the page follow.
	pub fn resize(&mut self, size: PhysicalSize<u32>) {
		if size.width == 0 || size.height == 0 {
			return;
		}
		self.config.width = size.width;
		self.config.height = size.height;
		if let Some(surface) = &self.surface {
			surface.configure(&self.gpu.device, &self.config);
		}
		self.target = RenderTarget::new(&self.gpu, size.width, size.height);
		let scale = self.window().scale_factor();
		let (width, height) = css_size(size, scale);
		self.host().resize(width, height, scale);
		// The new surface shows nothing yet.
		self.drawn_paints = u64::MAX;
	}

	/// Draw the page's latest paint and present it.
	pub fn draw(&mut self) {
		self.drawn_paints = self.host().paints();
		self.target.clear(&self.gpu);
		self.layer.draw(&self.gpu, &self.target);
		let Some(surface) = &self.surface else { return };
		let texture = match surface.get_current_texture() {
			wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
			wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
				surface.configure(&self.gpu.device, &self.config);
				self.drawn_paints = u64::MAX;
				return;
			}
			_ => {
				self.drawn_paints = u64::MAX;
				return;
			}
		};
		let view = texture.texture.create_view(&wgpu::TextureViewDescriptor::default());
		let rect = (0, 0, self.config.width, self.config.height);
		self.presenter.draw(&self.gpu, &self.target, &view, rect);
		self.window().pre_present_notify();
		self.gpu.queue.present(texture);
	}

	/// Save the frame as last drawn to `path` (a PNG).
	pub fn save(&self, path: &Path) -> Result<(), String> {
		let image = self.target.read(&self.gpu)?;
		save_png(path.to_string_lossy().as_ref(), &image)
	}

	/// The pointer moved to `(x, y)` in the window's pixels.
	pub fn mouse_moved(&mut self, x: f64, y: f64) {
		let scale = self.window().scale_factor();
		self.pointer = ((x / scale) as i32, (y / scale) as i32);
		self.host().mouse_move(self.pointer.0, self.pointer.1);
	}

	pub fn mouse_button(&mut self, button: winit::event::MouseButton, down: bool) {
		let button = match button {
			winit::event::MouseButton::Left => MouseButton::Left,
			winit::event::MouseButton::Right => MouseButton::Right,
			winit::event::MouseButton::Middle => MouseButton::Middle,
			_ => return,
		};
		self.host().mouse_button(self.pointer.0, self.pointer.1, button, down);
	}

	/// The wheel turned `notches` (up positive).
	pub fn wheel(&mut self, notches: i32) {
		if notches != 0 {
			self.host().wheel(self.pointer.0, self.pointer.1, notches);
		}
	}

	pub fn set_modifiers(&mut self, state: ModifiersState) {
		self.modifiers = Modifiers {
			shift: state.shift_key(),
			control: state.control_key(),
			alt: state.alt_key(),
			command: cfg!(target_os = "macos") && state.super_key(),
		};
		self.host().set_modifiers(self.modifiers);
	}

	/// A key went down or up: as the game's page gets it (its Windows key code), and what
	/// it typed.
	pub fn key(&mut self, event: &winit::event::KeyEvent) {
		let winit::keyboard::PhysicalKey::Code(code) = event.physical_key else { return };
		let down = event.state == winit::event::ElementState::Pressed;
		if let Some(engine) = crate::keys::code_for(code, &event.logical_key) {
			self.host().key(bb_ui::host::coherent_key_code(engine), down, self.modifiers);
		}
		if down {
			for c in event.text.iter().flat_map(|t| t.chars()) {
				self.host().character(c, self.modifiers);
			}
		}
	}

	/// Why the page can't be shown, if Chromium painted it where it can't be (in
	/// memory: its GPU compositing failed).
	pub fn paint_failure(&self) -> Option<String> {
		self.host()
			.painted_in_memory()
			.then(|| requirements::page_error(format!("{}: Chromium's GPU compositing failed", bb_ui::host::NO_PAINTS)))
	}
}

impl Drop for View {
	/// The page goes (CEF shuts down), then the window. Under gamescope's Vulkan layer,
	/// destroying the surface and the window crashed in libwayland (see the game's
	/// `Running::close`): on Linux both are left to the process's end, hidden. Elsewhere
	/// they go, so a window doesn't stay up while the relaunched game runs (Windows,
	/// where the first process waits for it).
	fn drop(&mut self) {
		drop(self.host.take());
		let (surface, window) = (self.surface.take(), self.window.take());
		if let Some(window) = &window {
			window.set_visible(false);
		}
		if cfg!(all(unix, not(target_os = "macos"))) {
			std::mem::forget(surface);
			std::mem::forget(window);
		} else {
			drop(surface);
			drop(window);
		}
	}
}

/// The page's size in CSS pixels for a window of `size` pixels at `scale`.
fn css_size(size: PhysicalSize<u32>, scale: f64) -> (i32, i32) {
	let css = |px: u32| ((f64::from(px) / scale).round() as i32).max(1);
	(css(size.width), css(size.height))
}
/// The window's `HWND` on Windows (the page's dialogs' parent); `None` elsewhere.
fn win32_handle(window: &Window) -> Option<isize> {
	use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
	match window.window_handle().ok()?.as_raw() {
		RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
		_ => None,
	}
}

