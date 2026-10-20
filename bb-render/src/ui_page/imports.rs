//! The browser's paint buffers imported once each, not once per paint.
//!
//! Chromium paints into a small pool of buffers (3 with CEF 154) and hands them over
//! in turn, so most paints come in a buffer we have imported before. CEF's rule is that
//! a buffer may only be *used* during its paint's callback and must not be cached; its
//! fd may be closed once the callback returns. An import here holds its own duplicate
//! of the fd, so the buffer can't be freed or replaced under it, and it is only read
//! (copied from) during a callback that hands over that same buffer. Keeping the import
//! across paints therefore never touches a buffer outside its callback, and saves
//! importing it (a Vulkan image and a memory import, ~37 µs) and destroying it again
//! each paint, plus wgpu's bookkeeping for a new texture in every copy: the paint
//! handler's CPU time went from 139 to 100 µs a paint.
//!
//! A buffer is known by its DMA-BUF's inode (unique per buffer on the system while it
//! exists, and ours keeps it existing), checked against the paint's description (size,
//! stride, offset, modifier): a mismatch imports anew. Imports not handed over for
//! [`STALE_AFTER`] paints are dropped (the pool changed, e.g. after a resize), and at
//! most [`MAX`] are kept, so a buffer the browser let go of is only kept alive a few
//! paints longer.

use std::fs::File;
use std::mem::ManuallyDrop;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::MetadataExt;

use super::linux::DmaBuf;

/// The most imports kept: the browser's pool (3), and one more for a pool in transition.
const MAX: usize = 4;

/// Paints after which an import that wasn't handed over again is dropped.
const STALE_AFTER: u64 = 8;

/// What identifies a buffer and its layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Key {
	dev: u64,
	ino: u64,
	width: u32,
	height: u32,
	stride: u32,
	offset: u64,
	modifier: u64,
}

impl Key {
	/// The key of `buffer`'s DMA-BUF (its inode, from `fstat` on the browser's fd).
	pub fn of(buffer: &DmaBuf) -> Result<Key, String> {
		// SAFETY: the browser's fd, open for the duration of the paint's callback;
		// wrapped only to `fstat` it and never dropped (closed) here.
		let file = ManuallyDrop::new(unsafe { File::from_raw_fd(buffer.fd.as_raw_fd()) });
		let meta = file.metadata().map_err(|e| format!("looking at the paint's buffer: {e}"))?;
		Ok(Key {
			dev: meta.dev(),
			ino: meta.ino(),
			width: buffer.width,
			height: buffer.height,
			stride: buffer.stride,
			offset: buffer.offset,
			modifier: buffer.modifier,
		})
	}
}

struct Import<T> {
	key: Key,
	texture: T,
	/// The paint count when it was last handed over.
	used: u64,
}

/// The imports kept, by buffer (`T`: the imported texture).
pub(super) struct Imports<T = wgpu::Texture> {
	kept: Vec<Import<T>>,
	paints: u64,
}

impl<T> Default for Imports<T> {
	fn default() -> Self {
		Imports { kept: Vec::new(), paints: 0 }
	}
}

impl<T: Clone> Imports<T> {
	/// The import of the buffer `key`, made by `import` if there is none yet; drops the
	/// stale ones (and the oldest beyond [`MAX`]).
	pub fn get(
		&mut self,
		key: Key,
		import: impl FnOnce() -> Result<T, String>,
	) -> Result<(T, bool), String> {
		self.paints += 1;
		let paints = self.paints;
		self.kept.retain(|i| i.key == key || (paints - i.used <= STALE_AFTER && i.key.width == key.width && i.key.height == key.height));
		if let Some(found) = self.kept.iter_mut().find(|i| i.key == key) {
			found.used = paints;
			return Ok((found.texture.clone(), false));
		}
		let texture = import()?;
		if self.kept.len() >= MAX {
			let oldest = (0..self.kept.len()).min_by_key(|&i| self.kept[i].used).expect("MAX > 0");
			self.kept.swap_remove(oldest);
		}
		self.kept.push(Import { key, texture: texture.clone(), used: paints });
		Ok((texture, true))
	}
}

#[cfg(test)]
mod tests {
	use super::{Imports, Key, MAX, STALE_AFTER};

	fn key(ino: u64, width: u32) -> Key {
		Key { dev: 1, ino, width, height: 10, stride: width * 4, offset: 0, modifier: 0 }
	}

	/// Hands over `ino` (at `width`): whether it was imported.
	fn paint(imports: &mut Imports<u64>, ino: u64, width: u32) -> bool {
		let (texture, imported) = imports.get(key(ino, width), || Ok(ino)).expect("imports");
		assert_eq!(texture, ino, "the import of that buffer");
		imported
	}

	#[test]
	fn a_pool_of_buffers_is_imported_once() {
		let mut imports = Imports::default();
		let imported: Vec<bool> = (0..30).map(|i| paint(&mut imports, i % 3, 100)).collect();
		assert_eq!(imported.iter().filter(|&&i| i).count(), 3);
		assert!(imported[..3].iter().all(|&i| i));
	}

	#[test]
	fn another_layout_of_the_same_buffer_is_imported_anew() {
		let mut imports = Imports::default();
		assert!(paint(&mut imports, 7, 100));
		assert!(paint(&mut imports, 7, 120));
		assert!(!paint(&mut imports, 7, 120));
		assert_eq!(imports.kept.len(), 1, "a resize lets go of the old size's buffers");
	}

	#[test]
	fn buffers_the_browser_stopped_using_are_let_go() {
		let mut imports = Imports::default();
		for ino in 0..3 {
			paint(&mut imports, ino, 100);
		}
		for _ in 0..=STALE_AFTER {
			paint(&mut imports, 9, 100);
		}
		assert_eq!(imports.kept.len(), 1, "only the buffer still handed over is kept");
		for ino in 10..20 {
			paint(&mut imports, ino, 100);
		}
		assert!(imports.kept.len() <= MAX);
	}
}
