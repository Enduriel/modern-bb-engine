//! How the world map's water and fog textures (`gfx/water.png`, `gfx/fow.png`) are
//! kept on the GPU. Both are 2048² in the game's data and tile over the view at every
//! zoom, so they keep a full mip chain: as RGBA8, 21.3 MiB each.
//!
//! Their content allows less: the fog is
//! grey (red, green and blue alike; its pass reads no alpha), the water opaque RGB of
//! low contrast. `BB_WATER_TEXTURE` and `BB_FOG_TEXTURE` choose, for trying them in
//! play ([`Choice::parse`]):
//! - `rgba8` (the default): as the data has it, mips made on the GPU;
//! - `r8`: one channel (the fog; exact for a grey texture), a quarter of the bytes;
//! - `bc1` (colour) or `bc4` (one channel): block compressed, an eighth of RGBA8's
//!   bytes, encoded on the CPU with its mips (`crate::block`);
//! - `-half` after any of them: stored at half the size (each side), drawn as large;
//! - `-flat` after any of them: no mips.
//!
//! What the content or the GPU rules out falls back ([`resolve`]): a texture that isn't
//! grey (a mod's) to RGBA8 for `r8`/`bc4`; one with alpha to RGBA8 for the water's
//! `bc1`; a GPU without BC compression (or a size not in whole blocks) to `r8` where
//! the content allows it, else RGBA8. Scripts never see any of it.

mod levels;

use bb_assets::Image;

use crate::gpu::Gpu;
use crate::mips::MipMaker;
use crate::texture::GpuTexture;

/// Which world pass a texture is for: what its pass reads of it differs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pass {
	/// `water.fs`: red, green, blue and alpha.
	Water,
	/// `fow.fs`: red, green and blue (its alpha comes from the fog mask).
	Fog,
}

impl Pass {
	fn reads_alpha(self) -> bool {
		self == Pass::Water
	}
}

/// A texture format a world texture is kept in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Storage {
	Rgba8,
	/// One channel: red, read as grey.
	R8,
	/// Opaque colour, 4×4 blocks of 8 bytes.
	Bc1,
	/// One channel, read as grey, 4×4 blocks of 8 bytes.
	Bc4,
}

impl Storage {
	pub fn format(self) -> wgpu::TextureFormat {
		match self {
			Storage::Rgba8 => wgpu::TextureFormat::Rgba8Unorm,
			Storage::R8 => wgpu::TextureFormat::R8Unorm,
			Storage::Bc1 => wgpu::TextureFormat::Bc1RgbaUnorm,
			Storage::Bc4 => wgpu::TextureFormat::Bc4RUnorm,
		}
	}

	fn name(self) -> &'static str {
		match self {
			Storage::Rgba8 => "rgba8",
			Storage::R8 => "r8",
			Storage::Bc1 => "bc1",
			Storage::Bc4 => "bc4",
		}
	}
}

/// Whether a texture of `format` holds one channel, which a pass reads as grey.
pub fn is_grey(format: wgpu::TextureFormat) -> bool {
	matches!(format, wgpu::TextureFormat::R8Unorm | wgpu::TextureFormat::Bc4RUnorm)
}

/// How a world texture is asked to be kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Choice {
	pub storage: Storage,
	/// Stored at half its size each side.
	pub half: bool,
	/// With its mip chain.
	pub mips: bool,
}

impl Choice {
	/// As the data has it, with mips: the default.
	pub const DEFAULT: Choice = Choice {
		storage: Storage::Rgba8,
		half: false,
		mips: true,
	};

	/// `rgba8`, `r8`, `bc1` or `bc4`, then any of `-half` and `-flat`.
	pub fn parse(text: &str) -> Result<Choice, String> {
		let mut parts = text.trim().split('-');
		let storage = match parts.next().unwrap_or_default().to_ascii_lowercase().as_str() {
			"rgba8" => Storage::Rgba8,
			"r8" => Storage::R8,
			"bc1" => Storage::Bc1,
			"bc4" => Storage::Bc4,
			other => return Err(format!("{other:?} isn't a world texture format (rgba8, r8, bc1, bc4)")),
		};
		let mut choice = Choice { storage, ..Choice::DEFAULT };
		for part in parts {
			match part.to_ascii_lowercase().as_str() {
				"half" => choice.half = true,
				"flat" => choice.mips = false,
				other => return Err(format!("{other:?} isn't a world texture option (half, flat)")),
			}
		}
		Ok(choice)
	}

	/// The choice in the environment variable `name`; the default without it (or, said
	/// on stderr, with one it can't read).
	fn from_env(name: &str) -> Choice {
		let Some(value) = std::env::var_os(name) else { return Choice::DEFAULT };
		match Choice::parse(&value.to_string_lossy()) {
			Ok(choice) => choice,
			Err(e) => {
				eprintln!("{name}: {e}; using rgba8");
				Choice::DEFAULT
			}
		}
	}
}

/// The water's and the fog's choices, and whether the GPU decodes BC formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorldFormats {
	pub water: Choice,
	pub fog: Choice,
	pub bc: bool,
}

impl WorldFormats {
	/// The environment's choices (`BB_WATER_TEXTURE`, `BB_FOG_TEXTURE`) on `gpu`.
	pub fn from_env(gpu: &Gpu) -> WorldFormats {
		WorldFormats {
			water: Choice::from_env("BB_WATER_TEXTURE"),
			fog: Choice::from_env("BB_FOG_TEXTURE"),
			bc: gpu.device.features().contains(wgpu::Features::TEXTURE_COMPRESSION_BC),
		}
	}

	fn choice(&self, pass: Pass) -> Choice {
		match pass {
			Pass::Water => self.water,
			Pass::Fog => self.fog,
		}
	}
}

/// What an image's content allows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Content {
	/// Red, green and blue alike in every texel.
	pub grey: bool,
	/// Alpha 255 in every texel.
	pub opaque: bool,
}

impl Content {
	pub fn of(image: &Image) -> Content {
		let mut content = Content { grey: true, opaque: true };
		for texel in image.rgba.chunks_exact(4) {
			content.grey &= texel[0] == texel[1] && texel[1] == texel[2];
			content.opaque &= texel[3] == 255;
			if !content.grey && !content.opaque {
				break;
			}
		}
		content
	}
}

/// The format a texture for `pass` is kept in: `wanted`, unless its `content`, its
/// stored `size` or the GPU (`bc`: it decodes BC formats) rule that out.
pub fn resolve(pass: Pass, wanted: Storage, content: Content, size: (u32, u32), bc: bool) -> Storage {
	// What the pass reads of it has to survive: alpha (the water's), colour.
	let alpha_kept = content.opaque || !pass.reads_alpha();
	let one_channel = content.grey && alpha_kept;
	let blocks = bc && size.0 % 4 == 0 && size.1 % 4 == 0;
	let fallback = if one_channel { Storage::R8 } else { Storage::Rgba8 };
	match wanted {
		Storage::Rgba8 => Storage::Rgba8,
		Storage::R8 => fallback,
		Storage::Bc4 if one_channel && blocks => Storage::Bc4,
		Storage::Bc1 if alpha_kept && blocks => Storage::Bc1,
		Storage::Bc4 | Storage::Bc1 => fallback,
	}
}

/// Upload `image` (`path`'s) for `pass` as `formats` choose (see the module's docs).
/// The default is the original path: RGBA8 uploaded as is, its mips made on the GPU.
/// The texture keeps the image's size as its own (`width`, `height`), the size its
/// pass takes coordinates over, whatever size it's stored at.
pub(crate) fn upload(gpu: &Gpu, mips: &MipMaker, formats: WorldFormats, pass: Pass, path: &str, image: &Image) -> GpuTexture {
	let choice = formats.choice(pass);
	if choice == Choice::DEFAULT {
		return mips.with_mips(gpu, &GpuTexture::upload_flat(gpu, path, image));
	}
	let stored = if choice.half { bb_assets::mips::halve(image) } else { image.clone() };
	let storage = resolve(pass, choice.storage, Content::of(image), (stored.width, stored.height), formats.bc);
	if storage != choice.storage {
		eprintln!("{path}: {} isn't possible here, kept as {}", choice.storage.name(), storage.name());
	}
	let label = format!("{path} {}{}{}", storage.name(), if choice.half { " half" } else { "" }, if choice.mips { "" } else { " flat" });
	let texture = levels::upload(gpu, &label, &stored, storage, choice.mips);
	let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
	GpuTexture {
		texture,
		view,
		width: image.width,
		height: image.height,
	}
}

#[cfg(test)]
mod tests;
