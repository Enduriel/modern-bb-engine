//! The window: winit's event loop driving the [`Game`] a frame per redraw, with the
//! player's mouse and keys passed on, and each frame presented on the window's surface.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use bb_render::{Gpu, Presenter, RenderTarget, save_png};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::keyboard::PhysicalKey;
use winit::window::{CustomCursor, Window, WindowId};

use bb_app::front::Key;
use bb_app::game::Game;
use bb_app::keys;
use bb_app::wheel::{self, Notches};

/// What to start, from the command line.
pub struct Options {
	pub data_dir: PathBuf,
	pub seed: String,
	/// Save the frame after this many frames as a PNG and quit (`--shot N PATH`).
	pub shot: Option<(u64, PathBuf)>,
	/// No sound (`--mute`); otherwise it plays on the default output device.
	pub mute: bool,
}

/// Everything that exists once the window does.
struct Running {
	window: Arc<Window>,
	surface: wgpu::Surface<'static>,
	config: wgpu::SurfaceConfiguration,
	gpu: Gpu,
	presenter: Presenter,
	target: RenderTarget,
	game: Game,
	last_frame: Instant,
	frames: u64,
	/// Whether the system's cursor shows over the window (the game draws its own
	/// otherwise), and which, as last set.
	system_cursor: Option<(bool, String)>,
	/// The hardware cursors, by name (`<install>/cursors/*.cur`).
	cursors: HashMap<String, CustomCursor>,
	/// The wheel's turn short of a whole notch (see `wheel`).
	wheel: Notches,
}

/// The original's hardware cursors: each `<install>/cursors/<name>.cur` (the install
/// is above the data folder), keyed by name.
fn load_cursors(event_loop: &ActiveEventLoop, data_dir: &Path) -> HashMap<String, CustomCursor> {
	let Some(dir) = data_dir.parent().map(|d| d.join("cursors")) else { return HashMap::new() };
	let Ok(entries) = std::fs::read_dir(&dir) else { return HashMap::new() };
	let mut cursors = HashMap::new();
	for path in entries.flatten().map(|e| e.path()) {
		let Some(name) = path.file_stem().and_then(|s| s.to_str()).map(str::to_string) else { continue };
		let image = std::fs::read(&path).ok().and_then(|b| bb_assets::parse_cursor(&b));
		let Some(image) = image else {
			eprintln!("cursor {}: unreadable", path.display());
			continue;
		};
		let source = CustomCursor::from_rgba(image.rgba, image.width as u16, image.height as u16, image.hotspot.0, image.hotspot.1);
		match source {
			Ok(source) => {
				cursors.insert(name, event_loop.create_custom_cursor(source));
			}
			Err(e) => eprintln!("cursor {}: {e}", path.display()),
		}
	}
	cursors
}

pub struct App {
	options: Options,
	instance: wgpu::Instance,
	running: Option<Running>,
	/// The first error, which ends the loop.
	pub error: Option<String>,
}

impl App {
	pub fn new(options: Options, instance: wgpu::Instance) -> Self {
		App {
			options,
			instance,
			running: None,
			error: None,
		}
	}

	fn start(&mut self, event_loop: &ActiveEventLoop) -> Result<Running, String> {
		let attributes = Window::default_attributes().with_title("Battle Brothers");
		let window = Arc::new(event_loop.create_window(attributes).map_err(|e| e.to_string())?);
		let surface = self
			.instance
			.create_surface(window.clone())
			.map_err(|e| format!("no surface for the window: {e}"))?;
		let gpu = Gpu::for_surface(&self.instance, &surface)?;
		eprintln!("GPU: {}", gpu.adapter);
		let game = Game::boot(&gpu, &self.options.data_dir, &self.options.seed, !self.options.mute)?;
		// The window opens at the game's screen size.
		let (w, h) = game.screen();
		let size = window.request_inner_size(PhysicalSize::new(w, h)).unwrap_or(window.inner_size());
		let capabilities = gpu.surface_capabilities(&surface).ok_or("no surface capabilities")?;
		let format = capabilities
			.formats
			.iter()
			.copied()
			.find(|f| !f.is_srgb())
			.or(capabilities.formats.first().copied())
			.ok_or("the surface offers no format")?;
		let config = wgpu::SurfaceConfiguration {
			usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
			format,
			width: size.width.max(1),
			height: size.height.max(1),
			present_mode: wgpu::PresentMode::Fifo,
			desired_maximum_frame_latency: 2,
			// An opaque window where the compositor allows it (the frame's alpha is
			// written as 1 anyway).
			alpha_mode: if capabilities.alpha_modes.contains(&wgpu::CompositeAlphaMode::Opaque) {
				wgpu::CompositeAlphaMode::Opaque
			} else {
				capabilities.alpha_modes[0]
			},
			view_formats: Vec::new(),
			color_space: wgpu::SurfaceColorSpace::Auto,
		};
		surface.configure(&gpu.device, &config);
		let presenter = Presenter::new(&gpu, format);
		let target = RenderTarget::new(&gpu, config.width, config.height);
		Ok(Running {
			window,
			surface,
			config,
			gpu,
			presenter,
			target,
			game,
			last_frame: Instant::now(),
			frames: 0,
			system_cursor: None,
			cursors: load_cursors(event_loop, &self.options.data_dir),
			wheel: Notches::default(),
		})
	}

	/// Stop with `error`.
	fn fail(&mut self, event_loop: &ActiveEventLoop, error: String) {
		self.error.get_or_insert(error);
		event_loop.exit();
	}
}

impl Running {
	/// Window pixels to the game's screen pixels.
	fn to_screen(&self, x: f64, y: f64) -> (i64, i64) {
		let (w, h) = self.game.screen();
		let sx = f64::from(w) / f64::from(self.config.width);
		let sy = f64::from(h) / f64::from(self.config.height);
		((x * sx) as i64, (y * sy) as i64)
	}

	fn resize(&mut self, size: PhysicalSize<u32>) {
		if size.width == 0 || size.height == 0 {
			return;
		}
		self.config.width = size.width;
		self.config.height = size.height;
		self.surface.configure(&self.gpu.device, &self.config);
		self.target = RenderTarget::new(&self.gpu, size.width, size.height);
	}

	/// One frame: step the game by the real time since the last, draw, present.
	/// Returns the frame count.
	fn frame(&mut self) -> Result<u64, String> {
		let now = Instant::now();
		let ms = now.duration_since(self.last_frame).as_secs_f64() * 1000.0;
		self.last_frame = now;
		self.game.step(ms)?;
		// The game draws its cursor unless the scripts want the system's (the hardware
		// cursor setting), which shows `cursors/<name>.cur` (a name without one keeps
		// the last, as the original's); otherwise the system's is hidden.
		let choice = self.game.cursor_choice();
		if self.system_cursor.as_ref() != Some(&choice) {
			let (hardware, name) = &choice;
			if *hardware {
				if let Some(cursor) = self.cursors.get(name) {
					self.window.set_cursor(cursor.clone());
				}
			}
			self.window.set_cursor_visible(*hardware);
			self.system_cursor = Some(choice);
		}
		self.game.draw(&self.gpu, &self.target);
		let texture = match self.surface.get_current_texture() {
			wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
			wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
				self.surface.configure(&self.gpu.device, &self.config);
				return Ok(self.frames);
			}
			_ => return Ok(self.frames),
		};
		let view = texture.texture.create_view(&wgpu::TextureViewDescriptor::default());
		self.presenter.draw(&self.gpu, &self.target, &view);
		self.window.pre_present_notify();
		self.gpu.queue.present(texture);
		self.frames += 1;
		Ok(self.frames)
	}

	fn event(&mut self, event: &WindowEvent) -> Result<(), String> {
		match event {
			WindowEvent::CursorMoved { position, .. } => {
				let (x, y) = self.to_screen(position.x, position.y);
				self.game.mouse_moved(x, y)
			}
			WindowEvent::MouseInput { state, button, .. } => {
				let id = match button {
					MouseButton::Left => 1,
					MouseButton::Right => 2,
					MouseButton::Middle => 3,
					MouseButton::Back => 4,
					MouseButton::Forward => 5,
					MouseButton::Other(_) => return Ok(()),
				};
				self.game.mouse_button(id, *state == ElementState::Pressed)
			}
			// Whole notches, each to the UI or the states (see `wheel`).
			WindowEvent::MouseWheel { delta, .. } => {
				let turned = self.wheel.add(wheel::notches(delta, self.window.scale_factor()));
				for _ in 0..turned.unsigned_abs() {
					self.game.wheel(turned > 0)?;
				}
				Ok(())
			}
			// A key held as the window loses the keyboard would never see its release.
			WindowEvent::Focused(false) => {
				self.game.keyboard_lost();
				Ok(())
			}
			WindowEvent::ModifiersChanged(m) => {
				self.game.set_modifier(keys::modifier(m.state()));
				Ok(())
			}
			WindowEvent::KeyboardInput { event, .. } => {
				let PhysicalKey::Code(code) = event.physical_key else { return Ok(()) };
				let down = event.state == ElementState::Pressed;
				self.game.key(&Key {
					engine: keys::code(code),
					windows: keys::windows_code(code),
					text: if down { event.text.as_ref().map(|t| t.to_string()) } else { None },
					down,
					repeat: event.repeat,
				})
			}
			_ => Ok(()),
		}
	}
}

impl ApplicationHandler for App {
	fn resumed(&mut self, event_loop: &ActiveEventLoop) {
		if self.running.is_some() {
			return;
		}
		// Wait, not Poll: a frame is drawn per redraw, and each frame requests the next
		// (`about_to_wait`). Where the compositor paces redraws (Wayland's frame callbacks),
		// Poll would spin the thread between them.
		event_loop.set_control_flow(ControlFlow::Wait);
		match self.start(event_loop) {
			Ok(running) => self.running = Some(running),
			Err(e) => self.fail(event_loop, e),
		}
	}

	fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
		let Some(running) = self.running.as_mut() else { return };
		let result = match &event {
			WindowEvent::CloseRequested => {
				event_loop.exit();
				Ok(())
			}
			WindowEvent::Resized(size) => {
				running.resize(*size);
				Ok(())
			}
			WindowEvent::RedrawRequested => running.frame().and_then(|frames| {
				match &self.options.shot {
					Some((after, path)) if frames >= *after => {
						let image = running.target.read(&running.gpu)?;
						save_png(path.to_string_lossy().as_ref(), &image)?;
						eprintln!("wrote {}", path.display());
						event_loop.exit();
					}
					_ => {}
				}
				Ok(())
			}),
			other => running.event(other),
		};
		if let Err(e) = result {
			self.fail(event_loop, e);
		}
	}

	fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
		if let Some(running) = &self.running {
			running.window.request_redraw();
		}
	}
}
