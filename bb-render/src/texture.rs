//! Uploading decoded images as sampled textures, with a full mip chain.
//!
//! The original's depth-of-field shader reads mip level 2 of every atlas
//! (`textureLod(.., 2)` in `sprite_dof.fs`), so atlases need mips. They are built on
//! the CPU with a 2×2 box filter over the stored (straight-alpha) RGBA, the usual
//! `glGenerateMipmap` result.
//!
//! Bitmap fonts have none: region names drawn at half size match the original's
//! stroke brightness only when sampled from the full-size atlas (screenshot: the
//! text's 95th-percentile brightness 138 in the original, 120 with
//! mips, 138 without).

use bb_assets::Image;

use crate::gpu::Gpu;

/// The format atlases are uploaded in.
pub const TEXTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// The bytes a 2D `texture` takes on the GPU, every mip level counted (what its
/// texels need; the driver's padding and alignment aren't known here). A block
/// compressed level takes whole blocks.
pub fn texture_bytes(texture: &wgpu::Texture) -> u64 {
	let size = texture.size();
	let format = texture.format();
	let block = u64::from(format.block_copy_size(None).unwrap_or(4));
	block_mip_bytes(size.width, size.height, texture.mip_level_count(), format.block_dimensions(), block)
}

/// The bytes of a `width` × `height` image of `texel`-byte texels with `levels` mip
/// levels (each half the last, rounded down, at least 1).
#[cfg(test)]
fn mip_bytes(width: u32, height: u32, levels: u32, texel: u64) -> u64 {
	block_mip_bytes(width, height, levels, (1, 1), texel)
}

/// [`mip_bytes`] for blocks of `(bw, bh)` texels and `block` bytes each.
fn block_mip_bytes(width: u32, height: u32, levels: u32, (bw, bh): (u32, u32), block: u64) -> u64 {
	(0..levels)
		.map(|level| {
			let (w, h) = ((width >> level).max(1), (height >> level).max(1));
			u64::from(w.div_ceil(bw)) * u64::from(h.div_ceil(bh)) * block
		})
		.sum()
}

/// A sampled texture on the GPU.
pub struct GpuTexture {
	pub texture: wgpu::Texture,
	pub view: wgpu::TextureView,
	pub width: u32,
	pub height: u32,
}

impl GpuTexture {
	/// The bytes it takes on the GPU (see [`texture_bytes`]).
	pub fn bytes(&self) -> u64 {
		texture_bytes(&self.texture)
	}

	/// Upload `image` with every mip level down to 1×1. Each level is made from the
	/// last, uploaded and dropped: at most two are alive at once, and `image` isn't
	/// copied (an atlas is up to 64 MiB).
	pub fn upload(gpu: &Gpu, label: &str, image: &Image) -> GpuTexture {
		Self::upload_levels(gpu, label, image, mip_count(image.width, image.height))
	}

	/// Upload `image` without mips.
	pub fn upload_flat(gpu: &Gpu, label: &str, image: &Image) -> GpuTexture {
		Self::upload_levels(gpu, label, image, 1)
	}

	/// Upload `image` and `levels - 1` halvings of it as its mip levels.
	fn upload_levels(gpu: &Gpu, label: &str, image: &Image, levels: u32) -> GpuTexture {
		let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
			label: Some(label),
			size: wgpu::Extent3d {
				width: image.width,
				height: image.height,
				depth_or_array_layers: 1,
			},
			mip_level_count: levels,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: TEXTURE_FORMAT,
			// COPY_SRC: level 0 is copied when its mips are made (`crate::mips`).
			usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
			view_formats: &[],
		});
		let write = |level: u32, mip: &Image| {
			gpu.queue.write_texture(
				wgpu::TexelCopyTextureInfo {
					texture: &texture,
					mip_level: level,
					origin: wgpu::Origin3d::ZERO,
					aspect: wgpu::TextureAspect::All,
				},
				&mip.rgba,
				wgpu::TexelCopyBufferLayout {
					offset: 0,
					bytes_per_row: Some(mip.width * 4),
					rows_per_image: Some(mip.height),
				},
				wgpu::Extent3d {
					width: mip.width,
					height: mip.height,
					depth_or_array_layers: 1,
				},
			);
		};
		write(0, image);
		let mut last: Option<Image> = None;
		for level in 1..levels {
			let next = bb_assets::mips::halve(last.as_ref().unwrap_or(image));
			write(level, &next);
			last = Some(next);
		}
		let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
		GpuTexture {
			texture,
			view,
			width: image.width,
			height: image.height,
		}
	}
}

/// How many mip levels a `width` × `height` image has, down to 1×1 (as many as
/// [`crate::mip_chain`] makes).
pub(crate) fn mip_count(width: u32, height: u32) -> u32 {
	32 - width.max(height).max(1).leading_zeros()
}


#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn mips_add_about_a_third() {
		assert_eq!(mip_bytes(2, 2, 2, 4), 20, "2×2 and 1×1");
		assert_eq!(mip_bytes(4, 1, 3, 4), 28, "4×1, 2×1, 1×1");
		assert_eq!(mip_bytes(2048, 2048, 12, 4), 22_369_620, "an atlas: 16 MiB and a third");
		// BC: 8 bytes a 4×4 block; the levels under 4×4 take a whole block each.
		assert_eq!(block_mip_bytes(2048, 2048, 12, (4, 4), 8), 2_796_216, "an eighth, and three tail blocks");
		assert_eq!(block_mip_bytes(4, 4, 3, (4, 4), 8), 24);
	}


	#[test]
	fn the_mip_count_is_the_chains_length() {
		for (width, height) in [(1, 1), (8, 2), (5, 3), (2048, 1024), (4096, 4096), (1, 7)] {
			let image = Image {
				width,
				height,
				rgba: vec![0; (width * height * 4) as usize],
			};
			assert_eq!(mip_count(width, height) as usize, crate::mip_chain(&image).len(), "{width}×{height}");
		}
	}

}
