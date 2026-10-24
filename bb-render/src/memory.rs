//! What the renderer holds, for the memory inventory: every texture on
//! the GPU, by path, with its bytes, and the warmed images waiting on the CPU for a draw
//! (see [`crate::residency`]). A decoded image is dropped once uploaded.

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

/// One of the renderer's pages of frames (`crate::pages`), for a memory snapshot.
#[derive(Clone, Debug, PartialEq)]
pub struct PageMemory {
	pub page: usize,
	pub width: u32,
	pub height: u32,
	pub bytes: u64,
	/// The frames in it.
	pub frames: usize,
	/// The last frame (the renderer's count) drawn from it.
	pub last_drawn: u64,
}

/// A texture's residency, for a memory snapshot: the frame (the renderer's count) it was
/// last drawn at, and who ever drew it (see `crate::residency`).
#[derive(Clone, Debug, PartialEq)]
pub struct TextureDrawn {
	pub path: String,
	pub last_drawn: u64,
	pub drawers: String,
}

/// What the renderer's residency knows of its textures and pages, for a memory
/// snapshot: the frame now, the budget, each texture's last draw, and each page.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ResidencyDetail {
	pub frame: u64,
	pub budget: u64,
	pub textures: Vec<TextureDrawn>,
	pub pages: Vec<PageMemory>,
	/// The shaped text's glyph pages (`TextShaper`): how many, and their GPU bytes (not
	/// in [`RendererMemory`]).
	pub glyph_pages: (usize, u64),
}

/// The renderer's GPU memory, as plain data.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RendererMemory {
	/// The textures loaded by path, largest first.
	pub textures: Vec<TextureMemory>,
	/// Paths still loading, and paths that failed with nothing in their place.
	pub loading: usize,
	pub failed: usize,
	/// Warmed images waiting on the CPU for a draw: how many, and their bytes.
	pub held: usize,
	pub held_bytes: u64,
	/// Our pages of frames (`crate::pages`): how many, how many frames in them, their bytes.
	pub pages: usize,
	pub frames: usize,
	pub page_bytes: u64,
	/// The fog of war's mask.
	pub fog_mask: u64,
	/// The UI page's texture, when the game draws one (not the renderer's own).
	pub ui_page: u64,
	/// The renderer's own textures (a white texel, the sprite program's blank).
	pub own: u64,
}

/// One line: the textures (and how many stand in for missing ones), the fog mask, the
/// UI page, the renderer's own, and the paths loading or failed. Sizes in MiB.
impl std::fmt::Display for RendererMemory {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		let mib = |bytes: u64| bytes as f64 / (1024.0 * 1024.0);
		let stand_ins = self.textures.iter().filter(|t| t.stand_in).count();
		write!(
			f,
			"{:.1} MiB on the GPU: {} frames in {} pages {:.1} MiB, {} textures {:.1} MiB ({stand_ins} standing \
			in for missing ones), fog mask {:.1} MiB, UI page {:.1} MiB, own {:.1} MiB; {} loading, {} failed; \
			{} warmed waiting for a draw ({:.1} MiB on the CPU)",
			mib(self.total()),
			self.frames,
			self.pages,
			mib(self.page_bytes),
			self.textures.len(),
			mib(self.texture_bytes()),
			mib(self.fog_mask),
			mib(self.ui_page),
			mib(self.own),
			self.loading,
			self.failed,
			self.held,
			mib(self.held_bytes),
		)
	}
}

impl RendererMemory {
	/// The textures loaded by path, in bytes.
	pub fn texture_bytes(&self) -> u64 {
		self.textures.iter().map(|t| t.bytes).sum()
	}

	/// Everything counted, in bytes.
	pub fn total(&self) -> u64 {
		self.texture_bytes() + self.page_bytes + self.fog_mask + self.ui_page + self.own
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
				Slot::Decoded(image, _) => {
					memory.held += 1;
					memory.held_bytes += image.rgba.len() as u64;
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
