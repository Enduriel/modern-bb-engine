//! The world map's fog of war: the original's `fow` program, ported to WGSL.
//!
//! Undiscovered land is covered by a cloudy grey texture (`gfx/fow.png`). Where the
//! fog lies comes from the engine's fog mask (`squirrel_bb::FogPatch`), which
//! `uncoverFogOfWar` stamps clear. It spans the map's extent, and this pass takes its
//! opacity from it. As the original's pass (`0x4a24c0`) does,
//! it covers the screen with one quad:
//! - the mask is sampled at the world position over the map's extent;
//! - the fog texture is sampled at the world position over 2048 units across, its
//!   `v` scaled by the extent's aspect;
//! - as in `shaders/fow.fs`, three samples at different scales and drift directions
//!   are averaged, times the ambient colour, drifting with `Time.getRealTimeF() / 100`.
//!
//! Unlike the original, the quad's texture coordinates follow the view even where the
//! camera shows past its bounds (the original clamps them to the camera's limits).

use std::num::NonZeroU64;

use squirrel_bb::{FOG_MASK_SIZE, FogPatch};

use crate::gpu::Gpu;
use crate::target::{RenderTarget, TARGET_FORMAT};
use crate::texture::GpuTexture;
use crate::world::WorldView;

const SHADER: &str = r#"
struct Fog {
	center: vec2<f32>,
	half_extent: vec2<f32>,
	map_extent: vec2<f32>,
	factor: f32,
	_pad: f32,
	ambient: vec4<f32>,
}

@group(0) @binding(0) var<uniform> u: Fog;
@group(0) @binding(1) var fog_texture: texture_2d<f32>;
@group(0) @binding(2) var fog_sampler: sampler;
@group(0) @binding(3) var mask: texture_2d<f32>;
@group(0) @binding(4) var mask_sampler: sampler;

struct Out {
	@builtin(position) position: vec4<f32>,
	@location(0) fog_uv: vec2<f32>,
	@location(1) mask_uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> Out {
	// One triangle covering the screen.
	let ndc = vec2<f32>(f32((i << 1u) & 2u) * 2.0 - 1.0, f32(i & 2u) * 2.0 - 1.0);
	var out: Out;
	out.position = vec4<f32>(ndc, 0.0, 1.0);
	let world = u.center + ndc * u.half_extent;
	out.fog_uv = vec2<f32>(world.x, world.y * u.map_extent.x / u.map_extent.y) / 2048.0;
	// Mask rows run from the map's bottom, like texture rows from v = 0.
	out.mask_uv = world / u.map_extent;
	return out;
}

@fragment
fn fs_main(f: Out) -> @location(0) vec4<f32> {
	let k = u.factor;
	let uv = f.fog_uv;
	let c1 = textureSample(fog_texture, fog_sampler, uv + k * vec2<f32>(1.0, -0.25));
	let c2 = textureSample(fog_texture, fog_sampler, (uv + k * vec2<f32>(0.5, 1.5)) * 0.33);
	let c3 = textureSample(fog_texture, fog_sampler, (uv + k * vec2<f32>(1.25, 0.9)) * 1.75);
	let rgb = (c1.rgb * 0.33 + c2.rgb * 0.33 + c3.rgb * 0.33) * u.ambient.rgb;
	return vec4<f32>(rgb, textureSample(mask, mask_sampler, f.mask_uv).r);
}
"#;

/// The fog-of-war compositing pass, and the mask it reads.
pub struct FogPass {
	pipeline: wgpu::RenderPipeline,
	layout: wgpu::BindGroupLayout,
	uniforms: wgpu::Buffer,
	repeat: wgpu::Sampler,
	clamp: wgpu::Sampler,
	/// The engine's fog mask, once a patch arrived.
	mask: Option<(wgpu::Texture, wgpu::TextureView)>,
}

impl FogPass {
	pub fn new(gpu: &Gpu) -> Self {
		let device = &gpu.device;
		let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
			label: Some("fog"),
			source: wgpu::ShaderSource::Wgsl(SHADER.into()),
		});
		let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
			binding,
			visibility: wgpu::ShaderStages::FRAGMENT,
			ty: wgpu::BindingType::Texture {
				sample_type: wgpu::TextureSampleType::Float { filterable: true },
				view_dimension: wgpu::TextureViewDimension::D2,
				multisampled: false,
			},
			count: None,
		};
		let sampler_entry = |binding| wgpu::BindGroupLayoutEntry {
			binding,
			visibility: wgpu::ShaderStages::FRAGMENT,
			ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
			count: None,
		};
		let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("fog"),
			entries: &[
				wgpu::BindGroupLayoutEntry {
					binding: 0,
					visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
					ty: wgpu::BindingType::Buffer {
						ty: wgpu::BufferBindingType::Uniform,
						has_dynamic_offset: false,
						min_binding_size: NonZeroU64::new(48),
					},
					count: None,
				},
				texture_entry(1),
				sampler_entry(2),
				texture_entry(3),
				sampler_entry(4),
			],
		});
		let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
			label: Some("fog"),
			bind_group_layouts: &[Some(&layout)],
			immediate_size: 0,
		});
		let blend = wgpu::BlendComponent {
			src_factor: wgpu::BlendFactor::SrcAlpha,
			dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
			operation: wgpu::BlendOperation::Add,
		};
		let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("fog"),
			layout: Some(&pipeline_layout),
			vertex: wgpu::VertexState {
				module: &shader,
				entry_point: Some("vs_main"),
				compilation_options: Default::default(),
				buffers: &[],
			},
			primitive: wgpu::PrimitiveState::default(),
			depth_stencil: None,
			multisample: wgpu::MultisampleState::default(),
			fragment: Some(wgpu::FragmentState {
				module: &shader,
				entry_point: Some("fs_main"),
				compilation_options: Default::default(),
				targets: &[Some(wgpu::ColorTargetState {
					format: TARGET_FORMAT,
					blend: Some(wgpu::BlendState {
						color: blend,
						alpha: blend,
					}),
					write_mask: wgpu::ColorWrites::ALL,
				})],
			}),
			multiview_mask: None,
			cache: None,
		});
		let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
			label: Some("fog uniforms"),
			size: 48,
			usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
			mapped_at_creation: false,
		});
		let sampler = |label, mode| {
			device.create_sampler(&wgpu::SamplerDescriptor {
				label: Some(label),
				address_mode_u: mode,
				address_mode_v: mode,
				mag_filter: wgpu::FilterMode::Linear,
				min_filter: wgpu::FilterMode::Linear,
				mipmap_filter: wgpu::MipmapFilterMode::Linear,
				..Default::default()
			})
		};
		FogPass {
			pipeline,
			layout,
			uniforms,
			repeat: sampler("fog texture", wgpu::AddressMode::Repeat),
			clamp: sampler("fog mask", wgpu::AddressMode::ClampToEdge),
			mask: None,
		}
	}

	/// Copy a changed part of the engine's fog mask to the GPU.
	pub fn update_mask(&mut self, gpu: &Gpu, patch: &FogPatch) {
		let size = FOG_MASK_SIZE as u32;
		let (texture, _) = self.mask.get_or_insert_with(|| {
			let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
				label: Some("fog mask"),
				size: wgpu::Extent3d {
					width: size,
					height: size,
					depth_or_array_layers: 1,
				},
				mip_level_count: 1,
				sample_count: 1,
				dimension: wgpu::TextureDimension::D2,
				format: wgpu::TextureFormat::R8Unorm,
				usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
				view_formats: &[],
			});
			let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
			(texture, view)
		});
		if patch.width == 0
			|| patch.height == 0
			|| patch.x + patch.width > size
			|| patch.y + patch.height > size
		{
			return;
		}
		gpu.queue.write_texture(
			wgpu::TexelCopyTextureInfo {
				texture,
				mip_level: 0,
				origin: wgpu::Origin3d {
					x: patch.x,
					y: patch.y,
					z: 0,
				},
				aspect: wgpu::TextureAspect::All,
			},
			&patch.alpha,
			wgpu::TexelCopyBufferLayout {
				offset: 0,
				bytes_per_row: Some(patch.width),
				rows_per_image: Some(patch.height),
			},
			wgpu::Extent3d {
				width: patch.width,
				height: patch.height,
				depth_or_array_layers: 1,
			},
		);
	}

	/// Draw the fog over `target`: `fog` is `gfx/fow.png`, `map_extent` the world size
	/// the mask spans, `real_time` (seconds) drives the drift. Nothing is drawn before
	/// the mask's first patch.
	pub fn draw(
		&self,
		gpu: &Gpu,
		target: &RenderTarget,
		view: &WorldView,
		fog: &GpuTexture,
		map_extent: (f32, f32),
		real_time: f32,
	) {
		let Some((_, mask)) = &self.mask else { return };
		if !(map_extent.0 > 0.0 && map_extent.1 > 0.0) {
			return;
		}
		let half = [
			view.width as f32 / view.zoom / 2.0,
			view.height as f32 / view.zoom / 2.0,
		];
		let floats = [
			view.center[0],
			view.center[1],
			half[0],
			half[1],
			map_extent.0,
			map_extent.1,
			real_time / 100.0,
			0.0,
			1.0,
			1.0,
			1.0,
			1.0,
		];
		let bytes: Vec<u8> = floats.iter().flat_map(|f| f.to_le_bytes()).collect();
		gpu.queue.write_buffer(&self.uniforms, 0, &bytes);
		let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some("fog"),
			layout: &self.layout,
			entries: &[
				wgpu::BindGroupEntry {
					binding: 0,
					resource: self.uniforms.as_entire_binding(),
				},
				wgpu::BindGroupEntry {
					binding: 1,
					resource: wgpu::BindingResource::TextureView(&fog.view),
				},
				wgpu::BindGroupEntry {
					binding: 2,
					resource: wgpu::BindingResource::Sampler(&self.repeat),
				},
				wgpu::BindGroupEntry {
					binding: 3,
					resource: wgpu::BindingResource::TextureView(mask),
				},
				wgpu::BindGroupEntry {
					binding: 4,
					resource: wgpu::BindingResource::Sampler(&self.clamp),
				},
			],
		});
		let mut encoder = gpu
			.device
			.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
		{
			let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
				label: Some("fog"),
				color_attachments: &[Some(wgpu::RenderPassColorAttachment {
					view: &target.view,
					depth_slice: None,
					resolve_target: None,
					ops: wgpu::Operations {
						load: wgpu::LoadOp::Load,
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
	}
}
