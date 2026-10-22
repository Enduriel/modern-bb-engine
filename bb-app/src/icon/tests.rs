use super::{ICO, ico, images, res};

/// A pixel of the 48-pixel image: (x, y) from the top left, and its RGBA (read from the
/// original's icon with another decoder).
const PIXELS_48: [((usize, usize), [u8; 4]); 3] = [((0, 0), [255, 255, 255, 1]), ((24, 4), [229, 173, 70, 255]), ((24, 44), [5, 5, 4, 255])];

#[test]
fn the_original_icon_has_its_three_sizes() {
	let images = images();
	assert_eq!(images.iter().map(|i| i.size).collect::<Vec<_>>(), [16, 32, 48]);
	for image in &images {
		assert_eq!((image.bits, image.planes), (32, 1));
		assert_eq!(image.rgba().unwrap().len(), (image.size * image.size * 4) as usize);
	}
	let rgba = images[2].rgba().unwrap();
	for ((x, y), pixel) in PIXELS_48 {
		let at = (y * 48 + x) * 4;
		assert_eq!(rgba[at..at + 4], pixel, "the pixel at ({x}, {y}), top row first");
	}
}

#[test]
fn a_broken_icon_is_refused() {
	assert!(ico::images(b"not an icon").is_err());
	assert!(ico::images(&ICO[..100]).is_err(), "an image past the end");
	let mut png_entry = ICO.to_vec();
	let offset = u32::from_le_bytes(png_entry[18..22].try_into().unwrap()) as usize;
	png_entry[offset..offset + 8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
	assert!(ico::images(&png_entry).is_err(), "a PNG image");
}

/// A resource read back from a `.res` file: its type, id, language, flags and data.
struct Resource<'a> {
	kind: u16,
	id: u16,
	language: u16,
	flags: u16,
	data: &'a [u8],
}

/// The resources of the `.res` file `file`, read as a linker reads it.
fn read_res(file: &[u8]) -> Vec<Resource<'_>> {
	let u16_at = |at: usize| u16::from_le_bytes(file[at..at + 2].try_into().unwrap());
	let u32_at = |at: usize| u32::from_le_bytes(file[at..at + 4].try_into().unwrap()) as usize;
	let mut resources = Vec::new();
	let mut at = 0;
	while at < file.len() {
		let (size, header) = (u32_at(at), u32_at(at + 4));
		assert_eq!(header, 32, "numbered type and name");
		assert_eq!((u16_at(at + 8), u16_at(at + 12)), (0xFFFF, 0xFFFF));
		resources.push(Resource {
			kind: u16_at(at + 10),
			id: u16_at(at + 14),
			flags: u16_at(at + 20),
			language: u16_at(at + 22),
			data: &file[at + header..at + header + size],
		});
		at = (at + header + size).next_multiple_of(4);
	}
	assert_eq!(at, file.len());
	resources
}

#[test]
fn the_exe_resource_holds_the_icon_group() {
	let images = ico::images(ICO).unwrap();
	let file = res::icon_res(&images);
	let resources = read_res(&file);
	// The empty first one marks a 32-bit `.res` file.
	assert_eq!((resources[0].kind, resources[0].id, resources[0].data.len()), (0, 0, 0));
	let group = resources.iter().find(|r| r.kind == res::RT_GROUP_ICON).expect("the icon group");
	assert_eq!((group.id, group.language, group.flags), (res::GROUP_ID, 0x0409, 0x1030));
	let group = group.data;
	assert_eq!(group[..6], [0, 0, 1, 0, 3, 0]);
	for (i, image) in images.iter().enumerate() {
		let entry = &group[6 + 14 * i..6 + 14 * (i + 1)];
		// The `.ico`'s directory entry, its offset swapped for the image's resource id.
		assert_eq!(entry[..12], ICO[6 + 16 * i..6 + 16 * i + 12]);
		let id = u16::from_le_bytes([entry[12], entry[13]]);
		let icon = resources.iter().find(|r| r.kind == res::RT_ICON && r.id == id).expect("the image's resource");
		assert_eq!(icon.data, image.data);
		assert_eq!((icon.language, icon.flags), (0x0409, 0x1010));
	}
	assert_eq!(resources.len(), 2 + images.len());
}

/// Whether an opt-in window test may run: asked for (`BB_ICON_TEST=1`) and in a
/// display of its own (`var` set), never the player's.
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn opted_in(var: &str) {
	assert!(
		std::env::var_os("BB_ICON_TEST").is_some_and(|v| v == "1") && std::env::var_os(var).is_some(),
		"set BB_ICON_TEST=1 inside a display of its own ({var})"
	);
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
struct Ignore;

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
impl winit::application::ApplicationHandler for Ignore {
	fn resumed(&mut self, _: &winit::event_loop::ActiveEventLoop) {}
	fn window_event(&mut self, _: &winit::event_loop::ActiveEventLoop, _: winit::window::WindowId, _: winit::event::WindowEvent) {}
}

/// The X11 window's `_NET_WM_ICON` is the icon's largest image, read back with
/// `xprop`. In an X server of its own (`DISPLAY`: a headless gamescope's XWayland).
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
#[test]
#[ignore = "opt-in: BB_ICON_TEST=1, in an X server of its own"]
fn the_x11_window_has_the_icon() {
	use std::time::Duration;
	use winit::event_loop::EventLoop;
	use winit::platform::pump_events::EventLoopExtPumpEvents;
	use winit::platform::x11::EventLoopBuilderExtX11;
	use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

	opted_in("DISPLAY");
	let mut event_loop = EventLoop::builder().with_x11().with_any_thread(true).build().expect("an X11 event loop");
	#[allow(deprecated)]
	let window = event_loop.create_window(super::with_icon(winit::window::Window::default_attributes())).expect("a window");
	let _ = event_loop.pump_app_events(Some(Duration::from_millis(100)), &mut Ignore);
	let RawWindowHandle::Xlib(handle) = window.window_handle().unwrap().as_raw() else {
		panic!("not an X11 window");
	};
	let out = std::process::Command::new("xprop")
		.args(["-id", &handle.window.to_string(), "-notype", "-f", "_NET_WM_ICON", "32c", " = $0+", "_NET_WM_ICON"])
		.output()
		.expect("xprop");
	let text = String::from_utf8_lossy(&out.stdout);
	let values: Vec<u64> = text.split_once(" = ").expect(&text).1.split(',').map(|v| v.trim().parse().unwrap()).collect();
	assert_eq!(values[..2], [48, 48], "the largest image");
	assert_eq!(values.len(), 2 + 48 * 48);
	for ((x, y), [r, g, b, a]) in PIXELS_48 {
		let argb = u64::from(a) << 24 | u64::from(r) << 16 | u64::from(g) << 8 | u64::from(b);
		assert_eq!(values[2 + y * 48 + x], argb, "the pixel at ({x}, {y})");
	}
}

/// The Wayland window's toplevel gets the icon, and the compositor takes it (no
/// protocol error by a round trip after the window's commits). In a compositor of its
/// own with the protocol (`WAYLAND_DISPLAY`: a headless KWin, as the clipboard's test
/// runs one); with `BB_ICON_WAYLAND=none`, one without it (gamescope's), where the
/// window just has no icon. `BB_ICON_HOLD=SECONDS` keeps the window up that long, for
/// the compositor's side to be looked at: a KWin script's `window.icon` lists the
/// icon's sizes (16, 32, 48; checked 2026-10-22, KWin 6.6).
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
#[test]
#[ignore = "opt-in: BB_ICON_TEST=1, in a Wayland compositor of its own"]
fn the_wayland_toplevel_has_the_icon() {
	use std::time::Duration;
	use winit::event_loop::EventLoop;
	use winit::platform::pump_events::EventLoopExtPumpEvents;
	use winit::platform::wayland::EventLoopBuilderExtWayland;

	opted_in("WAYLAND_DISPLAY");
	let mut event_loop = EventLoop::builder().with_wayland().with_any_thread(true).build().expect("a Wayland event loop");
	#[allow(deprecated)]
	let window = event_loop.create_window(super::with_icon(winit::window::Window::default_attributes())).expect("a window");
	let display = event_loop.owned_display_handle();
	let icon = super::wayland::ToplevelIcon::set(&window, &display);
	if std::env::var_os("BB_ICON_WAYLAND").is_some_and(|v| v == "none") {
		assert!(icon.is_none(), "the compositor has no xdg_toplevel_icon_manager_v1");
		return;
	}
	let mut icon = icon.expect("the compositor's xdg_toplevel_icon_manager_v1");
	let _ = event_loop.pump_app_events(Some(Duration::from_millis(20)), &mut Ignore);
	let frame = icon.commit_frame(&window, 64);
	let _ = event_loop.pump_app_events(Some(Duration::from_millis(20)), &mut Ignore);
	icon.check().expect("the compositor took the icon");
	// To look at it from the compositor's side (its scripting) while it shows.
	if let Some(hold) = std::env::var("BB_ICON_HOLD").ok().and_then(|s| s.parse().ok()) {
		let until = std::time::Instant::now() + Duration::from_secs(hold);
		while std::time::Instant::now() < until {
			let _ = event_loop.pump_app_events(Some(Duration::from_millis(50)), &mut Ignore);
		}
	}
	drop(icon);
	icon_check_after_drop(&mut event_loop);
	drop(frame);
	drop(window);

	/// The icon's objects gone, the window keeps going (no error from the compositor).
	fn icon_check_after_drop(event_loop: &mut EventLoop<()>) {
		for _ in 0..3 {
			let status = event_loop.pump_app_events(Some(Duration::from_millis(20)), &mut Ignore);
			assert!(matches!(status, winit::platform::pump_events::PumpStatus::Continue), "winit's connection ended");
		}
	}
}
