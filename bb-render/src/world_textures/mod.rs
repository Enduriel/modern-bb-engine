//! How the world map's water and fog textures (`gfx/water.png`, `gfx/fow.png`) are
//! kept on the GPU. Both are 2048² in the game's data and tile over the view at every
//! zoom, so they keep a full mip chain: as RGBA8, 21.3 MiB each.
//!
//! Their content allows less: the fog is grey (red, green and blue alike; its pass reads
//! no alpha), the water opaque RGB of low contrast. So by default they're block
//! compressed: the water in BC1, the fog in BC4, 2.7 MiB each with mips (on screen at
//! most 4 levels of 255 off, PSNR 54–67 dB). Their levels are made and encoded on the
//! image loader's workers as they're decoded ([`LevelsAhead`]), so a load only copies them.
//! `BB_WATER_TEXTURE` and `BB_FOG_TEXTURE` choose another, for comparisons
//! ([`Choice::parse`]):
//! - `rgba8`: as the data has it, mips made on the GPU (21.3 MiB);
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

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use bb_assets::{Image, Prepare};

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

/// The water's texture.
pub(crate) const WATER_TEXTURE: &str = "gfx/water.png";

/// The fog of war's texture.
pub(crate) const FOG_TEXTURE: &str = "gfx/fow.png";

impl Pass {
	/// The pass `path` is the texture of, if either.
	pub(crate) fn of(path: &str) -> Option<Pass> {
		match path {
			WATER_TEXTURE => Some(Pass::Water),
			FOG_TEXTURE => Some(Pass::Fog),
			_ => None,
		}
	}

	/// How its texture is kept unless the environment says otherwise.
	pub fn default_choice(self) -> Choice {
		let storage = match self {
			Pass::Water => Storage::Bc1,
			Pass::Fog => Storage::Bc4,
		};
		Choice { storage, ..Choice::RGBA8 }
	}

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
	/// As the data has it, with mips (made on the GPU).
	pub const RGBA8: Choice = Choice {
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
		let mut choice = Choice { storage, ..Choice::RGBA8 };
		for part in parts {
			match part.to_ascii_lowercase().as_str() {
				"half" => choice.half = true,
				"flat" => choice.mips = false,
				other => return Err(format!("{other:?} isn't a world texture option (half, flat)")),
			}
		}
		Ok(choice)
	}

	/// The choice in the environment variable `name`; `default` without it (or, said
	/// on stderr, with one it can't read).
	fn from_env(name: &str, default: Choice) -> Choice {
		let Some(value) = std::env::var_os(name) else { return default };
		match Choice::parse(&value.to_string_lossy()) {
			Ok(choice) => choice,
			Err(e) => {
				eprintln!("{name}: {e}; using {}", default.storage.name());
				default
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
			water: Choice::from_env("BB_WATER_TEXTURE", Pass::Water.default_choice()),
			fog: Choice::from_env("BB_FOG_TEXTURE", Pass::Fog.default_choice()),
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

/// A texture's levels made ahead, and how they're kept.
pub(crate) struct Made {
	label: String,
	storage: Storage,
	levels: Vec<levels::Level>,
}

/// `image` (`path`'s) for `pass` as `formats` choose (see the module's docs), its levels
/// made on the CPU; `None` for RGBA8 at its size with mips (what it falls back to as
/// well), which [`upload`] makes on the GPU.
fn make(formats: WorldFormats, pass: Pass, path: &str, image: &Image) -> Option<Made> {
	let choice = formats.choice(pass);
	if choice == Choice::RGBA8 {
		return None;
	}
	let half = choice.half.then(|| bb_assets::mips::halve(image));
	let stored = half.as_ref().unwrap_or(image);
	let storage = resolve(pass, choice.storage, Content::of(image), (stored.width, stored.height), formats.bc);
	if storage != choice.storage {
		eprintln!("{path}: {} isn't possible here, kept as {}", choice.storage.name(), storage.name());
	}
	if storage == Storage::Rgba8 && !choice.half && choice.mips {
		return None;
	}
	let label = format!("{path} {}{}{}", storage.name(), if choice.half { " half" } else { "" }, if choice.mips { "" } else { " flat" });
	Some(Made {
		label,
		storage,
		levels: levels::make(stored, storage, choice.mips),
	})
}

/// The world textures' levels made on the image loader's workers as each is decoded,
/// by path, until its upload takes them: the encoding (~0.1–0.4 s of CPU for the pair)
/// runs beside the rest of a load, not on the frame that uploads.
#[derive(Clone, Default)]
pub(crate) struct LevelsAhead(Arc<Mutex<HashMap<String, Option<Made>>>>);

impl LevelsAhead {
	/// The loader's [`Prepare`] hook: the levels of a world texture as `formats` choose.
	pub(crate) fn hook(&self, formats: WorldFormats) -> Prepare {
		let made = self.0.clone();
		Arc::new(move |path: &str, image: &Image| {
			if let Some(pass) = Pass::of(path) {
				let levels = make(formats, pass, path, image);
				made.lock().unwrap_or_else(|e| e.into_inner()).insert(path.to_string(), levels);
			}
		})
	}

	fn take(&self, path: &str) -> Option<Option<Made>> {
		self.0.lock().unwrap_or_else(|e| e.into_inner()).remove(path)
	}
}

/// The world textures' formats and the levels made for them ahead: what the renderer's
/// uploads need.
#[derive(Clone)]
pub(crate) struct WorldTextures {
	pub formats: WorldFormats,
	pub prepared: LevelsAhead,
}

impl WorldTextures {
	/// Upload `path`'s `image` (see [`upload`]): a world texture as chosen, anything else
	/// as is (level 0 only).
	pub(crate) fn upload(&self, gpu: &Gpu, mips: &MipMaker, path: &str, image: &Image) -> GpuTexture {
		match Pass::of(path) {
			Some(pass) => upload(gpu, mips, self.formats, &self.prepared, pass, path, image),
			None => GpuTexture::upload_flat(gpu, path, image),
		}
	}
}

/// Upload `image` (`path`'s) for `pass` as `formats` choose (see the module's docs),
/// with the levels a worker made for it if it did (`prepared`), else made here. RGBA8
/// at its size is uploaded as is, its mips made on the GPU. The texture keeps the
/// image's size as its own (`width`, `height`), the size its pass takes coordinates
/// over, whatever size it's stored at.
pub(crate) fn upload(gpu: &Gpu, mips: &MipMaker, formats: WorldFormats, prepared: &LevelsAhead, pass: Pass, path: &str, image: &Image) -> GpuTexture {
	let made = prepared.take(path).unwrap_or_else(|| make(formats, pass, path, image));
	let Some(made) = made else {
		return mips.with_mips(gpu, &GpuTexture::upload_flat(gpu, path, image));
	};
	let texture = levels::upload(gpu, &made.label, &made.levels, made.storage);
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
