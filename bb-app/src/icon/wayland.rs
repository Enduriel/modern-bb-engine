//! The icon on a Wayland window: winit 0.30 sets none there (its `set_window_icon` does
//! nothing on Wayland), so it goes to the window's `xdg_toplevel` through the
//! compositor's `xdg_toplevel_icon_manager_v1` (the `xdg-toplevel-icon-v1` staging
//! protocol), each of the icon's sizes as a `wl_shm` buffer in one memfd.
//!
//! On winit's own connection (the toplevel is one of its objects), through a queue of
//! our own, as `smithay-clipboard` does for the clipboard: the `wl_display` from the
//! window's display handle, the toplevel from winit's `WindowExtWayland::xdg_toplevel`
//! (checked to be one by libwayland before it's used). A compositor without the
//! protocol (GNOME's, gamescope's) leaves the window with the desktop's default icon.
//!
//! The objects stay while the window shows the icon: the compositor sets it at the
//! toplevel's next commit (the window's next frame), and the buffers must outlive the
//! icon object.

use std::fs::File;
use std::io::Write;
use std::os::fd::{AsFd, FromRawFd};

use wayland_client::backend::{Backend, ObjectId};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_shm::{self, WlShm};
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, delegate_noop};
use wayland_protocols::xdg::shell::client::xdg_toplevel::XdgToplevel;
use wayland_protocols::xdg::toplevel_icon::v1::client::xdg_toplevel_icon_manager_v1::XdgToplevelIconManagerV1;
use wayland_protocols::xdg::toplevel_icon::v1::client::xdg_toplevel_icon_v1::XdgToplevelIconV1;
use winit::event_loop::OwnedDisplayHandle;
use winit::platform::wayland::WindowExtWayland;
use winit::raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
use winit::window::Window;

/// The icon set on a window, and what it's made of.
pub(super) struct ToplevelIcon {
	manager: XdgToplevelIconManagerV1,
	icon: XdgToplevelIconV1,
	buffers: Vec<WlBuffer>,
	pool: WlShmPool,
	#[cfg(test)]
	shm: WlShm,
	_memory: File,
	// The objects' queue: no event matters but a test's round trip.
	#[cfg_attr(not(test), allow(dead_code))]
	queue: EventQueue<State>,
	connection: Connection,
	// winit's connection, held open until the objects above are gone.
	_display: OwnedDisplayHandle,
}

/// The queue's state: nothing (no event matters).
struct State;

impl ToplevelIcon {
	/// Set the icon on `window`, if it is a Wayland window on `display` and the
	/// compositor has the protocol.
	pub(super) fn set(window: &Window, display: &OwnedDisplayHandle) -> Option<ToplevelIcon> {
		let RawDisplayHandle::Wayland(handle) = display.display_handle().ok()?.as_raw() else {
			return None;
		};
		let toplevel = window.xdg_toplevel()?;
		// SAFETY: `handle.display` is winit's `wl_display`, which `display` keeps
		// connected for as long as the icon lives (it is dropped last).
		let connection = Connection::from_backend(unsafe { Backend::from_foreign_display(handle.display.as_ptr().cast()) });
		let (globals, queue) = registry_queue_init::<State>(&connection).ok()?;
		let qh = queue.handle();
		let Ok(manager) = globals.bind::<XdgToplevelIconManagerV1, _, _>(&qh, 1..=1, ()) else {
			eprintln!("window icon: the compositor sets none (no xdg_toplevel_icon_manager_v1)");
			return None;
		};
		let shm: WlShm = globals.bind(&qh, 1..=1, ()).ok()?;
		// SAFETY: `toplevel` is the live window's `xdg_toplevel`, a proxy on the same
		// display; libwayland's class of it is checked to be `xdg_toplevel`.
		let id = unsafe { ObjectId::from_ptr(XdgToplevel::interface(), toplevel.as_ptr().cast()) }.ok()?;
		let toplevel = XdgToplevel::from_id(&connection, id).ok()?;

		let images: Vec<(u32, Vec<u8>)> = super::images()
			.iter()
			.filter_map(|image| Some((image.size, shm_pixels(&image.rgba().ok()?))))
			.collect();
		let total: usize = images.iter().map(|(_, pixels)| pixels.len()).sum();
		let mut memory = memfd(total)?;
		for (_, pixels) in &images {
			memory.write_all(pixels).ok()?;
		}
		let pool = shm.create_pool(memory.as_fd(), i32::try_from(total).ok()?, &qh, ());
		let icon = manager.create_icon(&qh, ());
		let mut offset = 0;
		let buffers = images
			.iter()
			.map(|(size, pixels)| {
				let side = *size as i32;
				let buffer = pool.create_buffer(offset, side, side, side * 4, wl_shm::Format::Argb8888, &qh, ());
				icon.add_buffer(&buffer, 1);
				offset += pixels.len() as i32;
				buffer
			})
			.collect();
		manager.set_icon(&toplevel, Some(&icon));
		let _ = connection.flush();
		Some(ToplevelIcon {
			manager,
			icon,
			buffers,
			pool,
			#[cfg(test)]
			shm,
			_memory: memory,
			queue,
			connection,
			_display: display.clone(),
		})
	}

	/// Whether the compositor took the requests: a round trip, which a protocol error
	/// (the compositor refusing one) fails.
	#[cfg(test)]
	pub(super) fn check(&mut self) -> Result<(), String> {
		self.queue.roundtrip(&mut State).map(|_| ()).map_err(|e| e.to_string())
	}

	/// Show `window` (a frame of `side` pixels, grey, committed on its `wl_surface`),
	/// which applies the icon, as the game's first frame does. What it returns holds
	/// the frame.
	#[cfg(test)]
	pub(super) fn commit_frame(&mut self, window: &Window, side: i32) -> impl Drop + use<> {
		use wayland_client::protocol::wl_surface::WlSurface;
		use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

		let RawWindowHandle::Wayland(handle) = window.window_handle().unwrap().as_raw() else {
			panic!("not a Wayland window");
		};
		// SAFETY: the live window's `wl_surface`, on the same display; its class is checked.
		let id = unsafe { ObjectId::from_ptr(WlSurface::interface(), handle.surface.as_ptr().cast()) }.unwrap();
		let surface = WlSurface::from_id(&self.connection, id).unwrap();
		let len = (side * side * 4) as usize;
		let mut memory = memfd(len).unwrap();
		memory.write_all(&vec![0x80; len]).unwrap();
		let qh = self.queue.handle();
		let pool = self.shm.create_pool(memory.as_fd(), len as i32, &qh, ());
		let buffer = pool.create_buffer(0, side, side, side * 4, wl_shm::Format::Argb8888, &qh, ());
		surface.attach(Some(&buffer), 0, 0);
		surface.damage_buffer(0, 0, side, side);
		surface.commit();
		let _ = self.connection.flush();
		struct Frame {
			_memory: File,
			pool: WlShmPool,
			buffer: WlBuffer,
		}
		impl Drop for Frame {
			fn drop(&mut self) {
				self.buffer.destroy();
				self.pool.destroy();
			}
		}
		Frame { _memory: memory, pool, buffer }
	}
}

impl Drop for ToplevelIcon {
	/// The icon object first (the window keeps its icon), then its buffers.
	fn drop(&mut self) {
		self.icon.destroy();
		for buffer in &self.buffers {
			buffer.destroy();
		}
		self.pool.destroy();
		self.manager.destroy();
		let _ = self.connection.flush();
	}
}

/// `rgba` (straight alpha) as `wl_shm`'s ARGB8888: each pixel a little-endian word,
/// so B, G, R, A in memory, premultiplied by its alpha as Wayland's buffers are.
fn shm_pixels(rgba: &[u8]) -> Vec<u8> {
	let mut out = Vec::with_capacity(rgba.len());
	for pixel in rgba.chunks_exact(4) {
		let alpha = u32::from(pixel[3]);
		let times = |c: u8| ((u32::from(c) * alpha + 127) / 255) as u8;
		out.extend_from_slice(&[times(pixel[2]), times(pixel[1]), times(pixel[0]), pixel[3]]);
	}
	out
}

/// A memory file of `len` bytes, to share with the compositor.
fn memfd(len: usize) -> Option<File> {
	// SAFETY: a valid name; the descriptor returned is new and owned by the file.
	let fd = unsafe { libc::memfd_create(c"bb-icon".as_ptr(), libc::MFD_CLOEXEC) };
	if fd < 0 {
		return None;
	}
	let file = unsafe { File::from_raw_fd(fd) };
	file.set_len(len as u64).ok()?;
	Some(file)
}

impl Dispatch<WlRegistry, GlobalListContents> for State {
	fn event(_: &mut State, _: &WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<State>) {}
}

delegate_noop!(State: ignore WlShm);
delegate_noop!(State: WlShmPool);
delegate_noop!(State: ignore WlBuffer);
delegate_noop!(State: ignore XdgToplevelIconManagerV1);
delegate_noop!(State: XdgToplevelIconV1);

#[cfg(test)]
mod tests {
	use super::shm_pixels;

	#[test]
	fn pixels_go_premultiplied_in_shm_order() {
		let rgba = [255, 128, 0, 255, 200, 100, 50, 128, 9, 9, 9, 0];
		assert_eq!(shm_pixels(&rgba), [0, 128, 255, 255, 25, 50, 100, 128, 0, 0, 0, 0]);
	}
}
