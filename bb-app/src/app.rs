//! The window: winit's event loop driving the [`Game`] a frame per redraw, with the
//! player's mouse and keys passed on, and each frame presented on the window's surface.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bb_render::{Gpu, Presenter, RenderTarget, save_png};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::keyboard::{ModifiersState, PhysicalKey};
use winit::window::{CustomCursor, Window, WindowId};

use bb_app::display::{self, Placement};
use bb_app::front::Key;
use bb_app::game::{Boot, Game, Start, Store};
use bb_app::steam::SteamStats;
use bb_app::settings::Settings;
use bb_app::wheel::{self, Notches};
use bb_app::pacing::Pacer;
use bb_app::{keys, platform};
use squirrel_bb::VideoMode;

use crate::window_mode::{self, Monitor};

/// What to start, from the command line.
pub struct Options {
	pub data_dir: PathBuf,
	/// The main menu, or a new campaign started at once (`--new-campaign`, `--seed`).
	pub start: Start,
	/// The install's mods mounted (`--mods`).
	pub mods: bool,
	/// Save the frame after this many frames as a PNG and quit (`--shot N PATH`).
	pub shot: Option<(u64, PathBuf)>,
	/// No sound (`--mute`); otherwise it plays on the default output device.
	pub mute: bool,
	/// The world map's columns and rows (`--world-size 280x280`); vanilla's otherwise.
	pub world_size: Option<(i64, i64)>,
	/// Serve Chrome DevTools for the page on localhost at this port (`--devtools PORT`,
	/// with the `ui` feature).
	pub devtools: Option<u16>,
	/// The settings file (`--settings PATH`); the system's place for it otherwise.
	pub settings: Option<PathBuf>,
	/// Squirrel to run after so many frames (`--at FRAMES CODE`).
	pub at: Vec<(u64, String)>,
	/// Where saves go (`--saves DIR`); the system's place for them otherwise
	/// (`squirrel_bb::platform::saves_dir`), or memory without one.
	pub saves: Option<PathBuf>,
	/// Whether the install is Steam's (the scripts' `isSteamBuild()`).
	pub steam_build: bool,
	/// Steam's API, started on a Steam install (see `bb_app::steam`); achievements off
	/// without it.
	pub steam: Option<Rc<SteamStats>>,
	/// Where CEF writes its log (beside log.html, see `main`); without one, in its
	/// temporary profile.
	pub cef_log: Option<PathBuf>,
	/// The FPS display shown from the start (`--fps`); otherwise as the settings say.
	pub fps: bool,
}

/// Everything that exists once the window does.
struct Running {
	window: Arc<Window>,
	surface: wgpu::Surface<'static>,
	config: wgpu::SurfaceConfiguration,
	gpu: Gpu,
	presenter: Presenter,
	/// The present modes the surface offers.
	present_modes: Vec<wgpu::PresentMode>,
	/// The frame, drawn at the size it shows at, and where in the window it shows.
	target: RenderTarget,
	frame_rect: (u32, u32, u32, u32),
	/// Where the game's screen goes (see `display`), and whether the system has
	/// exclusive fullscreen.
	placement: Placement,
	exclusive: bool,
	/// The scripts' video mode changes (`setVideoMode`) put on the window so far.
	applied: u64,
	game: Game,
	last_frame: Instant,
	frames: u64,
	/// The last frame showed what the one before it did, so nothing was drawn or
	/// presented (see `bb_app::shown`), and what paces such frames.
	unchanged: bool,
	pacer: Pacer,
	/// The refresh interval of the monitor the window is on, as the system last
	/// reported it, if it did.
	reported_refresh: Option<Duration>,
	/// When the redraw now awaited was asked for (see `App::about_to_wait`).
	redraw_asked: Option<Instant>,
	/// With `BB_TEST_UNPRESENTED=N`, frame N is drawn but never presented, as a present
	/// that came back out of date: the window tests' check that the game draws on
	/// without the compositor's answer (see `REDRAW_STALLED`).
	unpresented: Option<u64>,
	/// With `BB_TEST_PRESENT_EVERY=N`, two frames in a row of every N are drawn and
	/// presented, changed or not, as a mouse moved now and then does with the game's
	/// cursor: the window tests' check that undrawn frames keep the display's pace.
	present_every: Option<u64>,
	/// With `BB_STATS`, when the renderer's memory is logged next, and the frames since
	/// the last log: how many, and their own time (stepping, drawing, presenting).
	next_stats: Option<Instant>,
	stats_frames: (u32, Duration),
	/// Since the last log, the frames left undrawn because they showed what the last did.
	stats_unchanged: u32,
	/// Since the last log, the frames that drew sprites without their atlas (still
	/// loading), and those sprites.
	stats_waiting: (u32, usize),
	/// Whether the system's cursor shows over the window (the game draws its own
	/// otherwise), and which, as last set.
	system_cursor: Option<(bool, String)>,
	/// The hardware cursors, by name (`<install>/cursors/*.cur`).
	cursors: HashMap<String, CustomCursor>,
	/// The wheel's turn short of a whole notch (see `wheel`).
	wheel: Notches,
	/// The modifier keys held.
	modifiers: ModifiersState,
	/// Steam's API, whose callbacks run once a frame.
	steam: Option<Rc<SteamStats>>,
	/// With `BB_FRAME_TIMES=PATH`, each frame's time by span goes to that file (see
	/// `bb_render::frame_timing`); with `BB_FRAME_TIMES_GPU=1` too, each frame waits for
	/// the GPU, as the span `gpu`.
	frame_times: Option<FrameTimes>,
}

/// The per-frame times file (`BB_FRAME_TIMES`).
struct FrameTimes {
	out: std::io::BufWriter<std::fs::File>,
	wait_gpu: bool,
}

impl FrameTimes {
	fn from_env() -> Option<FrameTimes> {
		let path = std::env::var_os("BB_FRAME_TIMES").filter(|p| !p.is_empty())?;
		let file = std::fs::File::create(&path).map_err(|e| eprintln!("BB_FRAME_TIMES: {e}")).ok()?;
		bb_render::frame_timing::enable();
		Some(FrameTimes {
			out: std::io::BufWriter::new(file),
			wait_gpu: std::env::var_os("BB_FRAME_TIMES_GPU").is_some_and(|v| v != "0"),
		})
	}

	/// One line: the frame's number, the time since the last frame's start, its own work,
	/// the camera's zoom, the quads drawn, then each span, all in milliseconds.
	fn write(&mut self, frame: u64, interval: f64, work: Duration, zoom: f32, quads: usize) {
		use std::io::Write;
		let ms = |d: Duration| d.as_secs_f64() * 1000.0;
		let mut line = format!("{frame} interval={interval:.3} work={:.3} zoom={zoom:.3} quads={quads}", ms(work));
		for (name, time) in bb_render::frame_timing::take() {
			line.push_str(&format!(" {}={:.3}", name.replace(' ', "_"), ms(time)));
		}
		let _ = writeln!(self.out, "{line}");
	}
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
	settings: Settings,
	/// The first error, which ends the loop.
	pub error: Option<String>,
}

impl Drop for App {
	// However the loop ended (see `Running::close`).
	fn drop(&mut self) {
		if let Some(running) = self.running.take() {
			running.close();
		}
	}
}

impl App {
	pub fn new(options: Options, instance: wgpu::Instance) -> Self {
		let settings = match &options.settings {
			Some(path) => Settings::load(Some(path.clone())),
			None => Settings::load_default(),
		};
		App {
			options,
			instance,
			running: None,
			settings,
			error: None,
		}
	}

	fn start(&mut self, event_loop: &ActiveEventLoop) -> Result<Running, String> {
		// The mode the settings keep, or the original's first start: fullscreen at the
		// monitor's size.
		let monitor = Monitor::primary(event_loop);
		let (width, height) = monitor.size;
		let mode = self
			.settings
			.video_mode(VideoMode::first_start(i64::from(width), i64::from(height), 32));
		let exclusive = platform::exclusive_fullscreen(event_loop);
		let placement = Placement::of(&mode, monitor.size, &monitor.modes, exclusive);
		let screen = (mode.width.max(1) as u32, mode.height.max(1) as u32);
		let attributes = window_mode::attributes(placement, &monitor, screen);
		let window = Arc::new(event_loop.create_window(attributes).map_err(|e| e.to_string())?);
		let surface = self
			.instance
			.create_surface(window.clone())
			.map_err(|e| format!("no surface for the window: {e}"))?;
		let gpu = Gpu::for_surface(&self.instance, &surface)?;
		eprintln!("GPU: {}", gpu.adapter);
		squirrel_bb::html_log::line(squirrel_bb::html_log::Class::Info, "Renderer", format!("Device: {}", gpu.adapter));
		let options = &self.options;
		let saves = options.saves.clone().or_else(squirrel_bb::platform::saves_dir);
		match &saves {
			Some(dir) => eprintln!("saves: {}", dir.display()),
			None => eprintln!("saves: in memory only (no folder for them)"),
		}
		let boot = Boot {
			data_dir: &options.data_dir,
			start: options.start.clone(),
			mods: options.mods,
			sound: !options.mute,
			world_size: options.world_size,
			mode,
			devtools: options.devtools,
			saves,
			cef_log: options.cef_log.clone(),
			store: Store {
				steam_build: options.steam_build,
				user_stats: options.steam.clone().map(|s| s as Rc<dyn squirrel_bb::UserStats>),
			},
		};
		let mut game = Game::start(&gpu, &boot)?;
		game.video().set_monitor_modes(Monitor::of_window(&window).resolutions());
		if std::env::var_os("BB_TEXTURE_USAGE").is_some_and(|v| v != "0") {
			game.track_texture_usage();
		}
		if let Some(mib) = self.settings.texture_budget_mib() {
			game.set_texture_budget(mib << 20);
		}
		if options.fps || self.settings.show_fps() {
			game.show_fps(true)?;
		}
		let size = window.inner_size();
		eprintln!(
			"video: {}x{} {placement:?} on a {}x{} monitor, the window {}x{} (scale {})",
			mode.width, mode.height, monitor.size.0, monitor.size.1, size.width, size.height, window.scale_factor()
		);
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
			present_mode: display::present_mode(mode.vsync, &capabilities.present_modes),
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
		let frame_rect = display::frame_rect((config.width, config.height), game.screen());
		let target = RenderTarget::new(&gpu, frame_rect.2, frame_rect.3);
		let pacer = Pacer::new(None, config.present_mode == wgpu::PresentMode::Fifo);
		let mut running = Running {
			window,
			surface,
			config,
			gpu,
			presenter,
			present_modes: capabilities.present_modes.clone(),
			target,
			frame_rect,
			placement,
			exclusive,
			applied: 0,
			game,
			last_frame: Instant::now(),
			frames: 0,
			unchanged: false,
			pacer,
			reported_refresh: None,
			redraw_asked: None,
			unpresented: std::env::var("BB_TEST_UNPRESENTED").ok().and_then(|v| v.parse().ok()),
			present_every: std::env::var("BB_TEST_PRESENT_EVERY").ok().and_then(|v| v.parse().ok()).filter(|&n| n > 0),
			next_stats: std::env::var_os("BB_STATS").is_some_and(|v| v != "0").then(Instant::now),
			stats_frames: (0, Duration::ZERO),
			stats_unchanged: 0,
			stats_waiting: (0, 0),
			system_cursor: None,
			cursors: load_cursors(event_loop, &self.options.data_dir),
			wheel: Notches::default(),
			modifiers: ModifiersState::empty(),
			steam: self.options.steam.clone(),
			frame_times: FrameTimes::from_env(),
		};
		running.resize(size);
		running.follow_monitor_refresh();
		Ok(running)
	}

	/// Stop with `error`.
	fn fail(&mut self, event_loop: &ActiveEventLoop, error: String) {
		self.error.get_or_insert(error);
		self.quit(event_loop);
	}

	/// Leave the loop.
	fn quit(&mut self, event_loop: &ActiveEventLoop) {
		if let Some(running) = self.running.take() {
			running.close();
		}
		event_loop.exit();
	}
}

impl Running {
	/// Take it down at exit: the game goes, but the window and the surface drawn to it
	/// are left to the process's end. Under gamescope's Vulkan layer
	/// (`ENABLE_GAMESCOPE_WSI`, on in every game gamescope runs), destroying the surface
	/// also destroys the window's own `wl_surface`, before its role (traced:
	/// `wl_surface.destroy` among the swapchain's teardown, then winit's
	/// `xdg_toplevel.destroy`), and the window's teardown then crashes on the freed
	/// object in libwayland. The window can't go first: the surface must not outlive it.
	fn close(self) {
		if let Some(usage) = self.game.texture_usage() {
			report_usage(&usage, &self.game.renderer_memory());
		}
		let Running { window, surface, game, .. } = self;
		drop(game);
		std::mem::forget(surface);
		std::mem::forget(window);
	}

	/// Pace undrawn frames no faster than the refresh of the monitor the window is on,
	/// where the system reports it (on Wayland, not before the window has shown on one).
	fn follow_monitor_refresh(&mut self) {
		let reported = display::refresh_interval(&self.window);
		if reported != self.reported_refresh {
			let ms = |r: Duration| format!("{:.2} ms", r.as_secs_f64() * 1000.0);
			eprintln!("display: the monitor's refresh {}", reported.map_or("not reported".into(), ms));
			self.reported_refresh = reported;
			self.pacer.set_reported(reported);
		}
	}

	/// Window pixels to the game's screen pixels.
	fn to_screen(&self, x: f64, y: f64) -> (i64, i64) {
		let (w, h) = self.game.screen();
		let (left, top, width, height) = self.frame_rect;
		let sx = f64::from(w) / f64::from(width.max(1));
		let sy = f64::from(h) / f64::from(height.max(1));
		(((x - f64::from(left)) * sx) as i64, ((y - f64::from(top)) * sy) as i64)
	}

	/// The window is `size` now; in a window, the game's screen is too.
	fn resize(&mut self, size: PhysicalSize<u32>) {
		if size.width == 0 || size.height == 0 {
			return;
		}
		self.config.width = size.width;
		self.config.height = size.height;
		self.surface.configure(&self.gpu.device, &self.config);
		// The new surface shows nothing yet.
		self.game.forget_shown();
		if self.placement.follows_window() {
			self.game.video().set_screen_size(i64::from(size.width), i64::from(size.height));
		}
	}

	/// Put the scripts' video mode on the window when they set one (`setVideoMode`), and
	/// keep the mode they ask to keep (`Settings.save`) in `settings`.
	fn follow_video(&mut self, settings: &mut Settings) {
		let video = self.game.video();
		if let Some(mode) = video.take_save() {
			if let Err(e) = settings.save_video_mode(&mode) {
				eprintln!("settings: {e}");
			}
		}
		let changes = video.changes();
		if changes == self.applied {
			return;
		}
		self.applied = changes;
		let mode = video.mode();
		let monitor = Monitor::of_window(&self.window);
		let placement = Placement::of(&mode, monitor.size, &monitor.modes, self.exclusive);
		if placement != self.placement {
			eprintln!("video: {}x{} {placement:?}", mode.width, mode.height);
			self.placement = placement;
			let screen = (mode.width.max(1) as u32, mode.height.max(1) as u32);
			if let Some(size) = window_mode::apply(&self.window, placement, &monitor, screen) {
				self.resize(size);
			}
		}
		let present_mode = display::present_mode(mode.vsync, &self.present_modes);
		if present_mode != self.config.present_mode {
			self.config.present_mode = present_mode;
			self.surface.configure(&self.gpu.device, &self.config);
			self.game.forget_shown();
			self.pacer.set_display_paced(present_mode == wgpu::PresentMode::Fifo);
		}
	}

	/// When the next frame may start: with the frame rate limited (VSync off), a frame
	/// cap's interval after the last; after a frame left undrawn, which nothing paced,
	/// the display's refresh or the cap's interval after it (see `bb_app::pacing`).
	fn next_frame_at(&self) -> Option<Instant> {
		let mode = self.game.video().mode();
		let cap = (mode.limit_frame_rate && mode.frame_cap > 0).then(|| Duration::from_secs_f64(1.0 / mode.frame_cap as f64));
		let interval = if self.unchanged { Some(self.pacer.interval(cap)) } else { cap };
		interval.map(|i| self.last_frame + i)
	}

	/// One frame: step the game by the real time since the last, draw, present.
	/// Returns the frame count.
	fn frame(&mut self) -> Result<u64, String> {
		let now = Instant::now();
		let ms = now.duration_since(self.last_frame).as_secs_f64() * 1000.0;
		self.last_frame = now;
		if let Some(steam) = &self.steam {
			steam.run_callbacks();
		}
		self.game.step(ms)?;
		// Beside `stats`' log, what the renderer holds (every 5 s, with `BB_STATS`).
		if self.next_stats.is_some_and(|at| now >= at) {
			let frame = self.target.bytes() as f64 / (1024.0 * 1024.0);
			let allocator = self.gpu.allocator_use().map_or(String::new(), |(used, reserved)| {
				format!("; wgpu's allocator {:.1} MiB in {:.1} MiB of blocks", used as f64 / 1048576.0, reserved as f64 / 1048576.0)
			});
			eprintln!("renderer: {}; the frame {frame:.1} MiB{allocator}", self.game.renderer_memory());
			let (frames, work) = std::mem::take(&mut self.stats_frames);
			if frames > 0 {
				let ms = work.as_secs_f64() * 1000.0 / f64::from(frames);
				let (waited, sprites) = std::mem::take(&mut self.stats_waiting);
				let unchanged = std::mem::take(&mut self.stats_unchanged);
				eprintln!(
					"frames: {frames}, {ms:.2} ms of work each (stepping, drawing, presenting); \
					{unchanged} showed what the last did (not drawn); \
					{waited} drew without {sprites} sprites whose frame or atlas was still loading"
				);
			}
			self.next_stats = Some(now + Duration::from_secs(5));
		}
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
		// The frame at the size it shows at in the window.
		self.frame_rect = display::frame_rect((self.config.width, self.config.height), self.game.screen());
		let (_, _, width, height) = self.frame_rect;
		if (self.target.width, self.target.height) != (width, height) {
			self.target = RenderTarget::new(&self.gpu, width, height);
		}
		// A frame that would show just what the last did is neither drawn nor presented:
		// the last stays up, and the next frame comes a refresh of the display after it
		// (`pacing`), which is measured from presented frames: until it is, all are.
		if self.pacer.measuring() || self.present_every.is_some_and(|n| self.frames % n < 2) {
			self.game.forget_shown();
		}
		let drawn = self.game.draw_if_changed(&self.gpu, &self.target);
		let after_presented = !self.unchanged;
		self.unchanged = drawn.is_none();
		let Some(drawn) = drawn else {
			if let Some(times) = &mut self.frame_times {
				times.write(self.frames, ms, now.elapsed(), self.game.world_zoom(), 0);
			}
			if self.next_stats.is_some() {
				self.stats_frames.0 += 1;
				self.stats_frames.1 += now.elapsed();
				self.stats_unchanged += 1;
			}
			self.frames += 1;
			return Ok(self.frames);
		};
		if drawn.waiting > 0 {
			self.stats_waiting.0 += 1;
			self.stats_waiting.1 += drawn.waiting;
		}
		let acquire_span = bb_render::frame_timing::span("acquire");
		let acquired = self.surface.get_current_texture();
		drop(acquire_span);
		let texture = match acquired {
			wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
			wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
				self.surface.configure(&self.gpu.device, &self.config);
				self.game.forget_shown();
				return Ok(self.frames);
			}
			_ => {
				self.game.forget_shown();
				return Ok(self.frames);
			}
		};
		let view = texture.texture.create_view(&wgpu::TextureViewDescriptor::default());
		let present_span = bb_render::frame_timing::span("present");
		self.presenter.draw(&self.gpu, &self.target, &view, self.frame_rect);
		self.window.pre_present_notify();
		if self.unpresented == Some(self.frames) {
			eprintln!("frame {} not presented (BB_TEST_UNPRESENTED)", self.frames);
			drop(texture);
		} else {
			self.gpu.queue.present(texture);
		}
		drop(present_span);
		if !after_presented {
			// The window may have moved to another monitor while nothing was presented.
			self.follow_monitor_refresh();
		}
		self.pacer.presented(now, after_presented);
		if let Some(times) = &mut self.frame_times {
			if times.wait_gpu {
				let _span = bb_render::frame_timing::span("gpu");
				let _ = self.gpu.device.poll(wgpu::PollType::wait_indefinitely());
			}
			let zoom = self.game.world_zoom();
			times.write(self.frames, ms, now.elapsed(), zoom, drawn.quads);
		}
		if self.next_stats.is_some() {
			self.stats_frames.0 += 1;
			self.stats_frames.1 += now.elapsed();
		}
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
				self.modifiers = m.state();
				self.game.set_modifier(keys::modifier(m.state()));
				Ok(())
			}
			WindowEvent::KeyboardInput { event, .. } => {
				let PhysicalKey::Code(code) = event.physical_key else { return Ok(()) };
				let down = event.state == ElementState::Pressed;
				// The FPS display's key; the game gets it too, as any other (nothing in
				// vanilla or the mods surveyed binds it).
				if down && !event.repeat && keys::toggles_fps(code, self.modifiers) {
					let shown = !self.game.fps_shown();
					self.game.show_fps(shown)?;
				}
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

/// How long a redraw asked for may take to come before the frame is drawn without it.
/// On Wayland, winit holds a redraw back until the compositor answers the last frame's
/// callback (asked for by `pre_present_notify`). The compositor may never answer it: it
/// needn't while the window isn't shown (the protocol's own words), and it can't when
/// the present didn't reach it (a present that came back out of date, which wgpu drops
/// silently, commits nothing). Without this the game stood still for good: its scripts,
/// its page and its quit (seen as "bb didn't exit" in the window tests, the main thread
/// idle in winit's poll). The frame drawn then commits the surface, which carries the
/// callback still asked for, and the compositor's pace takes over again. Well above a
/// refresh interval (16.7 ms at 60 Hz, 33 at 30), so a paced window never sees it.
const REDRAW_STALLED: Duration = Duration::from_millis(100);

impl App {
	/// The frame for a redraw: step, draw and present it, run the scripts due at it, put
	/// the scripts' video mode on the window, and quit when the game's over or the shot
	/// is taken.
	fn redraw(&mut self, event_loop: &ActiveEventLoop) {
		let Some(running) = self.running.as_mut() else { return };
		running.redraw_asked = None;
		let result = running
			.frame()
			.and_then(|frames| {
				for (_, code) in self.options.at.iter().filter(|(at, _)| *at == frames) {
					running.game.run_script(code)?;
				}
				running.follow_video(&mut self.settings);
				Ok(frames)
			})
			.and_then(|frames| match &self.options.shot {
				// The scripts finished their last state (the main menu's Quit, or Escape
				// there): the game is over, as the original's.
				_ if running.game.quit() => Ok(true),
				Some((after, path)) if frames >= *after => {
					let image = running.target.read(&running.gpu)?;
					save_png(path.to_string_lossy().as_ref(), &image)?;
					eprintln!("wrote {}", path.display());
					Ok(true)
				}
				_ => Ok(false),
			});
		match result {
			Ok(true) => self.quit(event_loop),
			Ok(false) => {}
			Err(e) => self.fail(event_loop, e),
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
		// Whether to quit now.
		let result = match &event {
			WindowEvent::CloseRequested => Ok(true),
			WindowEvent::Resized(size) => {
				running.resize(*size);
				Ok(false)
			}
			// What the system shows of the window may be gone: draw it again. It may be on
			// another monitor, with another refresh.
			WindowEvent::Occluded(false) | WindowEvent::ScaleFactorChanged { .. } => {
				running.game.forget_shown();
				running.follow_monitor_refresh();
				Ok(false)
			}
			WindowEvent::RedrawRequested => return self.redraw(event_loop),
			other => running.event(other).map(|()| false),
		};
		match result {
			Ok(true) => self.quit(event_loop),
			Ok(false) => {}
			Err(e) => self.fail(event_loop, e),
		}
	}

	fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
		let Some(running) = &mut self.running else { return };
		let now = Instant::now();
		match running.next_frame_at() {
			Some(at) if now < at => event_loop.set_control_flow(ControlFlow::WaitUntil(at)),
			_ => {
				let asked = *running.redraw_asked.get_or_insert(now);
				if now >= asked + REDRAW_STALLED {
					// The compositor never answered (see `REDRAW_STALLED`).
					self.redraw(event_loop);
					if let Some(running) = &mut self.running {
						let asked = Instant::now();
						running.redraw_asked = Some(asked);
						event_loop.set_control_flow(ControlFlow::WaitUntil(asked + REDRAW_STALLED));
						running.window.request_redraw();
					}
				} else {
					event_loop.set_control_flow(ControlFlow::WaitUntil(asked + REDRAW_STALLED));
					running.window.request_redraw();
				}
			}
		}
	}
}

/// With `BB_TEXTURE_USAGE=1`, at the end: how much of each atlas the session drew (in
/// RGBA8 bytes, without mips) and whether its mips were sampled, and the atlases loaded
/// but never drawn (for the texture residency design).
fn report_usage(usage: &[bb_render::AtlasUse], memory: &bb_render::RendererMemory) {
	let mib = |bytes: u64| bytes as f64 / (1024.0 * 1024.0);
	let (mut drawn, mut whole) = (0, 0);
	for atlas in usage {
		let full = u64::from(atlas.width) * u64::from(atlas.height) * 4;
		let part = atlas.drawn_texels * 4;
		drawn += part;
		whole += full;
		eprintln!(
			"texture usage: {} {}x{}: {:.1}% drawn, {:.1} of {:.1} MiB, mips {}",
			atlas.path,
			atlas.width,
			atlas.height,
			part as f64 * 100.0 / full as f64,
			mib(part),
			mib(full),
			if atlas.mips { "sampled" } else { "never sampled" }
		);
	}
	for texture in memory.textures.iter().filter(|t| !usage.iter().any(|a| a.path == t.path)) {
		// The water and the fog of war are drawn by passes of their own, tiled, the whole
		// picture; the fonts' and others' quads would have counted.
		let how = if ["gfx/water.png", "gfx/fow.png"].contains(&texture.path.as_str()) {
			"drawn whole by its own pass"
		} else {
			"loaded, never drawn"
		};
		eprintln!("texture usage: {} {}x{}: {how} ({:.1} MiB)", texture.path, texture.width, texture.height, mib(texture.bytes));
	}
	eprintln!("texture usage: in all {:.1} of {:.1} MiB drawn (RGBA8, without mips)", mib(drawn), mib(whole));
}
