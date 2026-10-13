//! Uploading decoded images as sampled textures, with a full mip chain.
//!
//! The original's depth-of-field shader reads mip level 2 of every atlas
//! (`textureLod(.., 2)` in `sprite_dof.fs`), so atlases need mips. They are built on
//! the CPU with a 2×2 box filter over the stored (straight-alpha) RGBA, the usual
//! `glGenerateMipmap` result.

use bb_assets::Image;

use crate::gpu::Gpu;

/// The format atlases are uploaded in.
pub const TEXTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// A sampled texture on the GPU.
pub struct GpuTexture {
	pub texture: wgpu::Texture,
	pub view: wgpu::TextureView,
	pub width: u32,
	pub height: u32,
}

impl GpuTexture {
	/// Upload `image` with every mip level down to 1×1.
	pub fn upload(gpu: &Gpu, label: &str, image: &Image) -> GpuTexture {
		let levels = mip_chain(image);
		let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
			label: Some(label),
			size: wgpu::Extent3d {
				width: image.width,
				height: image.height,
				depth_or_array_layers: 1,
			},
			mip_level_count: levels.len() as u32,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: TEXTURE_FORMAT,
			usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
			view_formats: &[],
		});
		for (level, mip) in levels.iter().enumerate() {
			gpu.queue.write_texture(
				wgpu::TexelCopyTextureInfo {
					texture: &texture,
					mip_level: level as u32,
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

/// `image` and each halving of it down to 1×1. An odd edge drops its last
/// row/column; each texel is the rounded mean of the 2×2 block it covers.
pub fn mip_chain(image: &Image) -> Vec<Image> {
	let mut levels = vec![image.clone()];
	while let Some(last) = levels.last().filter(|i| i.width > 1 || i.height > 1) {
		levels.push(halve(last));
	}
	levels
}

fn halve(src: &Image) -> Image {
	let width = (src.width / 2).max(1);
	let height = (src.height / 2).max(1);
	let mut rgba = Vec::with_capacity((width * height * 4) as usize);
	let texel = |x: u32, y: u32, c: usize| {
		let (x, y) = (x.min(src.width - 1), y.min(src.height - 1));
		u32::from(src.rgba[((y * src.width + x) * 4) as usize + c])
	};
	for y in 0..height {
		for x in 0..width {
			for c in 0..4 {
				let sum = texel(2 * x, 2 * y, c)
					+ texel(2 * x + 1, 2 * y, c)
					+ texel(2 * x, 2 * y + 1, c)
					+ texel(2 * x + 1, 2 * y + 1, c);
				rgba.push(((sum + 2) / 4) as u8);
			}
		}
	}
	Image {
		width,
		height,
		rgba,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn chain_halves_down_to_one_texel() {
		let image = Image {
			width: 8,
			height: 2,
			rgba: vec![0; 8 * 2 * 4],
		};
		let sizes: Vec<(u32, u32)> = mip_chain(&image)
			.iter()
			.map(|i| (i.width, i.height))
			.collect();
		assert_eq!(sizes, [(8, 2), (4, 1), (2, 1), (1, 1)]);
	}

	#[test]
	fn each_texel_averages_its_block_with_rounding() {
		#[rustfmt::skip]
		let image = Image { width: 2, height: 2, rgba: vec![
			0, 10, 255, 1,   1, 10, 255, 1,
			0, 10, 255, 0,   0, 11, 255, 0,
		] };
		assert_eq!(halve(&image).rgba, [0, 10, 255, 1]);
	}
}
