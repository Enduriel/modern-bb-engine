//! What the renderer holds, for the memory inventory: every texture it
//! loaded, by path, with its GPU bytes. Nothing is evicted yet, so this only grows
//! over a session. No CPU copies are kept: a decoded image is dropped once uploaded.

use bb_assets::{Slot, TextureCache};

use crate::texture::GpuTexture;

/// One texture loaded by path: an atlas, a font, the water's or the fog's picture.
#[derive(Clone, Debug, PartialEq)]
pub struct TextureMemory {
	pub path: String,
	pub width: u32,
	pub height: u32,
	/// GPU bytes, every mip level counted.
	pub bytes: u64,
	/// The "texture missing" picture, standing in for a path that failed to load.
	pub stand_in: bool,
}

/// The renderer's GPU memory, as plain data.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RendererMemory {
	/// The textures loaded by path, largest first.
	pub textures: Vec<TextureMemory>,
	/// Paths still loading, and paths that failed with nothing in their place.
	pub loading: usize,
	pub failed: usize,
	/// The fog of war's mask.
	pub fog_mask: u64,
	/// The UI page's texture, when the game draws one (not the renderer's own).
	pub ui_page: u64,
	/// The renderer's own textures (a white texel, the sprite program's blank).
	pub own: u64,
}

impl RendererMemory {
	/// The textures loaded by path, in bytes.
	pub fn texture_bytes(&self) -> u64 {
		self.textures.iter().map(|t| t.bytes).sum()
	}

	/// Everything counted, in bytes.
	pub fn total(&self) -> u64 {
		self.texture_bytes() + self.fog_mask + self.ui_page + self.own
	}

	/// The textures of `cache`, largest first, and its loading and failed paths.
	pub(crate) fn of_cache(cache: &TextureCache<GpuTexture>) -> RendererMemory {
		let mut memory = RendererMemory::default();
		for (path, slot) in cache.slots() {
			let (texture, stand_in) = match slot {
				Slot::Ready(texture) => (texture, false),
				Slot::Missing(texture, _) => (texture, true),
				Slot::Loading => {
					memory.loading += 1;
					continue;
				}
				Slot::Failed(_) => {
					memory.failed += 1;
					continue;
				}
			};
			memory.textures.push(TextureMemory {
				path: path.to_string(),
				width: texture.width,
				height: texture.height,
				bytes: texture.bytes(),
				stand_in,
			});
		}
		memory.textures.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.path.cmp(&b.path)));
		memory
	}
}
