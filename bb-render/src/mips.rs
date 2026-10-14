//! An atlas's mip levels, made on the GPU when a draw first samples them (see
//! [`crate::residency`]). Atlases are uploaded with level 0 only; a sprite
//! drawn smaller than its texels, or below the battle camera's level with the depth of
//! field on, samples the smaller levels, and the renderer then replaces the texture with
//! one that has them all, before that frame draws. The chain adds a third to a texture,
//! so an atlas only ever drawn at its size (or larger) never has it.
//!
//! Each level is the one above halved as the CPU chain does ([`crate::mip_chain`]): a
//! texel is the rounded mean of the 2×2 block it covers, coordinates past an odd edge
//! clamped to it. The two agree to the bit (tested below).

use crate::gpu::Gpu;
use crate::sprite::Quad;
use crate::texture::{GpuTexture, TEXTURE_FORMAT, mip_count};

/// Whether drawing `quad` (from an atlas of `atlas` texels) at `zoom` screen pixels per
/// world unit samples the atlas's mips: either of its edges is drawn shorter than the
/// texels it spans (squashed or turned, too), or it's below the battle camera's level
/// `dof_below` with the depth of field on (which blends in mip 2). A quad drawn at its
/// size, to a thousandth, doesn't count: the GPU would weigh the next level at about that.
pub(crate) fn samples_mips(quad: &Quad, zoom: f32, (width, height): (u32, u32), dof_below: Option<u32>) -> bool {
	if dof_below.is_some_and(|level| quad.level < level) {
		return true;
	}
	let edge = |a: usize, b: usize| {
		let (c, t) = (quad.corners, quad.uv);
		let screen = (c[b][0] - c[a][0]).hypot(c[b][1] - c[a][1]) * zoom;
		let texels = ((t[b][0] - t[a][0]) * width as f32).hypot((t[b][1] - t[a][1]) * height as f32);
		screen < texels * 0.999
	};
	edge(0, 1) || edge(1, 2)
}

const SHADER: &str = r"
@group(0) @binding(0) var src: texture_2d<f32>;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
	// One triangle over the target: (-1, -1), (3, -1), (-1, 3).
	let xy = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
	return vec4<f32>(xy * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
	let last = vec2<i32>(textureDimensions(src)) - 1;
	let d = vec2<i32>(p.xy) * 2;
	let sum = textureLoad(src, min(d, last), 0)
		+ textureLoad(src, min(d + vec2<i32>(1, 0), last), 0)
		+ textureLoad(src, min(d + vec2<i32>(0, 1), last), 0)
		+ textureLoad(src, min(d + vec2<i32>(1, 1), last), 0);
	// The bytes' sum, then (sum + 2) / 4 rounded down, as the CPU chain.
	return floor((round(sum * 255.0) + 2.0) / 4.0) / 255.0;
}
";

/// Makes textures' mip levels.
pub(crate) struct MipMaker {
	pipeline: wgpu::RenderPipeline,
	layout: wgpu::BindGroupLayout,
}

impl MipMaker {
	pub(crate) fn new(gpu: &Gpu) -> MipMaker {
		let device = &gpu.device;
		let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
			label: Some("mips"),
			source: wgpu::ShaderSource::Wgsl(SHADER.into()),
		});
		let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("mips"),
			entries: &[wgpu::BindGroupLayoutEntry {
				binding: 0,
				visibility: wgpu::ShaderStages::FRAGMENT,
				ty: wgpu::BindingType::Texture {
					sample_type: wgpu::TextureSampleType::Float { filterable: false },
					view_dimension: wgpu::TextureViewDimension::D2,
					multisampled: false,
				},
				count: None,
			}],
		});
		let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
			label: Some("mips"),
			bind_group_layouts: &[Some(&layout)],
			immediate_size: 0,
		});
		let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("mips"),
			layout: Some(&pipeline_layout),
			vertex: wgpu::VertexState {
				module: &shader,
				entry_point: Some("vs_main"),
				compilation_options: Default::default(),
				buffers: &[],
			},
			fragment: Some(wgpu::FragmentState {
				module: &shader,
				entry_point: Some("fs_main"),
				compilation_options: Default::default(),
				targets: &[Some(wgpu::ColorTargetState {
					format: TEXTURE_FORMAT,
					blend: None,
					write_mask: wgpu::ColorWrites::ALL,
				})],
			}),
			primitive: wgpu::PrimitiveState::default(),
			depth_stencil: None,
			multisample: wgpu::MultisampleState::default(),
			multiview_mask: None,
			cache: None,
		});
		MipMaker { pipeline, layout }
	}

	/// `texture` (level 0 only) again, with every mip level down to 1×1.
	pub(crate) fn with_mips(&self, gpu: &Gpu, texture: &GpuTexture) -> GpuTexture {
		let (width, height) = (texture.width, texture.height);
		let levels = mip_count(width, height);
		let size = wgpu::Extent3d {
			width,
			height,
			depth_or_array_layers: 1,
		};
		let mipped = gpu.device.create_texture(&wgpu::TextureDescriptor {
			label: Some("mipped"),
			size,
			mip_level_count: levels,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: TEXTURE_FORMAT,
			usage: wgpu::TextureUsages::TEXTURE_BINDING
				| wgpu::TextureUsages::COPY_DST
				| wgpu::TextureUsages::COPY_SRC
				| wgpu::TextureUsages::RENDER_ATTACHMENT,
			view_formats: &[],
		});
		let mut encoder = gpu
			.device
			.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("mips") });
		encoder.copy_texture_to_texture(texture.texture.as_image_copy(), mipped.as_image_copy(), size);
		let level_view = |level: u32| {
			mipped.create_view(&wgpu::TextureViewDescriptor {
				base_mip_level: level,
				mip_level_count: Some(1),
				..Default::default()
			})
		};
		for level in 1..levels {
			let (src, dst) = (level_view(level - 1), level_view(level));
			let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
				label: Some("mips"),
				layout: &self.layout,
				entries: &[wgpu::BindGroupEntry {
					binding: 0,
					resource: wgpu::BindingResource::TextureView(&src),
				}],
			});
			let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
				label: Some("mips"),
				color_attachments: &[Some(wgpu::RenderPassColorAttachment {
					view: &dst,
					depth_slice: None,
					resolve_target: None,
					ops: wgpu::Operations {
						load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
						store: wgpu::StoreOp::Store,
					},
				})],
				depth_stencil_attachment: None,
				timestamp_writes: None,
				occlusion_query_set: None,
				multiview_mask: None,
			});
			pass.set_pipeline(&self.pipeline);
			pass.set_bind_group(0, &group, &[]);
			pass.draw(0..3, 0..1);
		}
		gpu.queue.submit([encoder.finish()]);
		let view = mipped.create_view(&wgpu::TextureViewDescriptor::default());
		GpuTexture {
			texture: mipped,
			view,
			width,
			height,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn quad(corners: [[f32; 2]; 4], uv: [[f32; 2]; 4]) -> Quad {
		Quad {
			texture: 0,
			corners,
			uv,
			color: [255; 4],
			corner_colors: None,
			overlay: [0; 4],
			factor: 0.0,
			saturation: 1.0,
			flags: 0,
			level: 0,
		}
	}

	#[test]
	fn a_quad_drawn_smaller_than_its_texels_samples_mips() {
		// 64×32 texels of a 256×256 atlas.
		let uv = [[0.0, 0.0], [0.25, 0.0], [0.25, 0.125], [0.0, 0.125]];
		let square = |w: f32, h: f32| [[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]];
		let atlas = (256, 256);
		assert!(!samples_mips(&quad(square(64.0, 32.0), uv), 1.0, atlas, None), "at its size");
		assert!(!samples_mips(&quad(square(128.0, 64.0), uv), 1.0, atlas, None), "larger");
		assert!(samples_mips(&quad(square(64.0, 32.0), uv), 0.5, atlas, None), "zoomed out");
		assert!(samples_mips(&quad(square(64.0, 16.0), uv), 1.0, atlas, None), "squashed");
		// Turned a quarter: its first edge runs down the screen, still 64 texels long.
		let turned = [[0.0, 0.0], [0.0, 64.0], [-32.0, 64.0], [-32.0, 0.0]];
		assert!(!samples_mips(&quad(turned, uv), 1.0, atlas, None), "turned, at its size");
		// Below the battle camera with the depth of field on: mip 2 blends in.
		let low = Quad { level: 0, ..quad(square(64.0, 32.0), uv) };
		assert!(samples_mips(&low, 1.0, atlas, Some(1)));
		assert!(!samples_mips(&low, 1.0, atlas, Some(0)));
	}
	use crate::mip_chain;
	use crate::target::read_level;
	use bb_assets::Image;

	#[test]
	fn the_gpus_levels_are_the_cpus() {
		let gpu = match Gpu::headless() {
			Ok(gpu) => gpu,
			Err(e) => {
				eprintln!("skipping: {e}");
				return;
			}
		};
		// Odd edges, a one-texel-wide stretch, every byte value.
		for (width, height) in [(37, 21), (5, 1), (64, 64)] {
			let rgba: Vec<u8> = (0..width * height * 4).map(|i| (i * 7919 % 256) as u8).collect();
			let image = Image { width, height, rgba };
			let flat = GpuTexture::upload_flat(&gpu, "test", &image);
			let mipped = MipMaker::new(&gpu).with_mips(&gpu, &flat);
			let cpu = mip_chain(&image);
			assert_eq!(mipped.texture.mip_level_count() as usize, cpu.len());
			for (level, want) in cpu.iter().enumerate() {
				let got = read_level(&gpu, &mipped.texture, level as u32).expect("read back");
				assert!(got == *want, "{width}×{height}, level {level}");
			}
		}
	}
}
