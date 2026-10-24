//! A world texture's levels made on the CPU and uploaded in a format other than the
//! default's: one channel, block compressed, or without mips. Each level is the one
//! above halved as [`bb_assets::mips::halve`] does (the GPU's default path makes the
//! same), then converted; at most two levels are alive at once.

use bb_assets::Image;

use crate::block::{self, Plane};
use crate::gpu::Gpu;
use crate::texture::mip_count;

use super::Storage;

/// One level's texels, in a format's layout.
struct Level {
	width: u32,
	height: u32,
	bytes: Vec<u8>,
}

/// `image` as a texture in `storage`, with its mip chain if `mips`. A one-channel
/// storage keeps its red (the caller checked it's grey).
pub(super) fn upload(gpu: &Gpu, label: &str, image: &Image, storage: Storage, mips: bool) -> wgpu::Texture {
	let levels = if mips { mip_count(image.width, image.height) } else { 1 };
	let format = storage.format();
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
		format,
		usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
		view_formats: &[],
	});
	let write = |level: u32, data: Level| write_level(gpu, &texture, level, &data);
	let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(8);
	match storage {
		Storage::Rgba8 | Storage::Bc1 => {
			let convert = |image: &Image| Level {
				width: image.width,
				height: image.height,
				bytes: match storage {
					Storage::Bc1 => block::encode_bc1(&Plane::rgba(image), threads),
					_ => image.rgba.clone(),
				},
			};
			write(0, convert(image));
			let mut last: Option<Image> = None;
			for level in 1..levels {
				let next = bb_assets::mips::halve(last.as_ref().unwrap_or(image));
				write(level, convert(&next));
				last = Some(next);
			}
		}
		Storage::R8 | Storage::Bc4 => {
			let mut grey = Level {
				width: image.width,
				height: image.height,
				bytes: image.rgba.chunks_exact(4).map(|t| t[0]).collect(),
			};
			for level in 0..levels {
				if level > 0 {
					grey = halve_grey(&grey);
				}
				let bytes = match storage {
					Storage::Bc4 => block::encode_bc4(&Plane { width: grey.width, height: grey.height, bytes: &grey.bytes, stride: 1 }, threads),
					_ => grey.bytes.clone(),
				};
				write(level, Level { width: grey.width, height: grey.height, bytes });
			}
		}
	}
	texture
}

/// `level` halved, one channel: each texel the rounded mean of the 2×2 it covers,
/// coordinates past an odd edge clamped (as [`bb_assets::mips::halve`] per channel).
fn halve_grey(level: &Level) -> Level {
	let (width, height) = ((level.width / 2).max(1), (level.height / 2).max(1));
	let at = |x: u32, y: u32| u32::from(level.bytes[(y.min(level.height - 1) * level.width + x.min(level.width - 1)) as usize]);
	let mut bytes = Vec::with_capacity((width * height) as usize);
	for y in 0..height {
		for x in 0..width {
			let sum = at(2 * x, 2 * y) + at(2 * x + 1, 2 * y) + at(2 * x, 2 * y + 1) + at(2 * x + 1, 2 * y + 1);
			bytes.push(((sum + 2) / 4) as u8);
		}
	}
	Level { width, height, bytes }
}

/// Write `data` (laid out as the texture's format wants) into `level` of `texture`.
fn write_level(gpu: &Gpu, texture: &wgpu::Texture, level: u32, data: &Level) {
	let format = texture.format();
	let (bw, bh) = format.block_dimensions();
	let block_bytes = format.block_copy_size(None).expect("a colour format");
	let (across, down) = (data.width.div_ceil(bw), data.height.div_ceil(bh));
	gpu.queue.write_texture(
		wgpu::TexelCopyTextureInfo {
			texture,
			mip_level: level,
			origin: wgpu::Origin3d::ZERO,
			aspect: wgpu::TextureAspect::All,
		},
		&data.bytes,
		wgpu::TexelCopyBufferLayout {
			offset: 0,
			bytes_per_row: Some(across * block_bytes),
			rows_per_image: Some(down),
		},
		// A level smaller than a block is copied as the whole block.
		wgpu::Extent3d {
			width: across * bw,
			height: down * bh,
			depth_or_array_layers: 1,
		},
	);
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_grey_halving_is_the_rgba_ones_red() {
		for (width, height) in [(7, 5), (1, 3), (8, 8)] {
			let rgba: Vec<u8> = (0..width * height).flat_map(|i| {
				let v = (i * 97 % 256) as u8;
				[v, v, v, 255]
			}).collect();
			let image = Image { width, height, rgba };
			let grey = Level { width, height, bytes: image.rgba.chunks_exact(4).map(|t| t[0]).collect() };
			let (cpu, ours) = (bb_assets::mips::halve(&image), halve_grey(&grey));
			assert_eq!((cpu.width, cpu.height), (ours.width, ours.height));
			assert!(cpu.rgba.chunks_exact(4).map(|t| t[0]).eq(ours.bytes.iter().copied()), "{width}×{height}");
		}
	}
}
