//! Screen-covering passes that lay a texture over the map in world space: the
//! original's water (`0x4e37a0`) and fog of war (`0x4a24c0`).
//!
//! Both draw one quad over the screen. Its texture coordinates are the world position
//! over the texture's size, stretched along the map's shorter side by the map's
//! aspect: with a map wider than tall, `u = x / width` and
//! `v = y * extent.x / (extent.y * height)`. A second coordinate is the world
//! position over the map's extent, which the fog uses to read its mask. A pass's
//! fragment shader supplies the rest. Unlike the original, the coordinates follow the
//! view even where the camera shows past its bounds (the original clamps them to the
//! camera's limits).

use std::num::NonZeroU64;

use crate::gpu::Gpu;
use crate::target::TARGET_FORMAT;
use crate::texture::GpuTexture;
use crate::world::WorldView;

/// The uniforms, bindings and vertex stage every pass shares.
const COMMON: &str = r#"
struct Pass {
	center: vec2<f32>,
	half_extent: vec2<f32>,
	map_extent: vec2<f32>,
	texture_size: vec2<f32>,
	factor: f32,
	_pad0: f32,
	_pad1: f32,
	_pad2: f32,
	ambient: vec4<f32>,
}

@group(0) @binding(0) var<uniform> u: Pass;
@group(0) @binding(1) var tex: texture_2d<f32>;
@group(0) @binding(2) var tex_sampler: sampler;

struct Out {
	@builtin(position) position: vec4<f32>,
	@location(0) uv: vec2<f32>,
	@location(1) map_uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> Out {
	// One triangle covering the screen.
	let ndc = vec2<f32>(f32((i << 1u) & 2u) * 2.0 - 1.0, f32(i & 2u) * 2.0 - 1.0);
	var out: Out;
	out.position = vec4<f32>(ndc, 0.0, 1.0);
	let world = u.center + ndc * u.half_extent;
	let e = u.map_extent;
	let span = u.texture_size * vec2<f32>(min(1.0, e.x / e.y), min(1.0, e.y / e.x));
	out.uv = world / span;
	// Rows of a map-wide texture run from the map's bottom, like texture rows from v = 0.
	out.map_uv = world / e;
	return out;
}
"#;

/// The bindings a pass with a mask adds.
const MASK: &str = r#"
@group(0) @binding(3) var mask: texture_2d<f32>;
@group(0) @binding(4) var mask_sampler: sampler;
"#;

/// What one draw of a pass needs besides its textures.
pub(crate) struct PassParams<'a> {
	pub view: &'a WorldView,
	/// The map's extent in world units.
	pub map_extent: (f32, f32),
	/// The texture's size the coordinates are taken over.
	pub texture_size: (f32, f32),
	/// The drift factor (`v_factor`).
	pub factor: f32,
	pub ambient: [f32; 4],
}

/// One screen-covering world-space pass.
pub(crate) struct WorldPass {
	label: &'static str,
	pipeline: wgpu::RenderPipeline,
	layout: wgpu::BindGroupLayout,
	uniforms: wgpu::Buffer,
	repeat: wgpu::Sampler,
	clamp: wgpu::Sampler,
}

/// The uniform buffer's size.
const UNIFORM_SIZE: u64 = 64;

impl WorldPass {
	/// A pass running `fragment` (WGSL with an `fs_main(f: Out)` entry point). With
	/// `masked`, it also binds a map-wide mask (`mask`, `mask_sampler`).
	pub fn new(gpu: &Gpu, label: &'static str, fragment: &str, masked: bool) -> Self {
		let device = &gpu.device;
		let source = format!("{COMMON}{}{fragment}", if masked { MASK } else { "" });
		let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
			label: Some(label),
			source: wgpu::ShaderSource::Wgsl(source.into()),
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
		let mut entries = vec![
			wgpu::BindGroupLayoutEntry {
				binding: 0,
				visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
				ty: wgpu::BindingType::Buffer {
					ty: wgpu::BufferBindingType::Uniform,
					has_dynamic_offset: false,
					min_binding_size: NonZeroU64::new(UNIFORM_SIZE),
				},
				count: None,
			},
			texture_entry(1),
			sampler_entry(2),
		];
		if masked {
			entries.extend([texture_entry(3), sampler_entry(4)]);
		}
		let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some(label),
			entries: &entries,
		});
		let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
			label: Some(label),
			bind_group_layouts: &[Some(&layout)],
			immediate_size: 0,
		});
		// Both of the original's materials: glBlendFuncSeparate(GL_SRC_ALPHA,
		// GL_ONE_MINUS_SRC_ALPHA, GL_ONE, GL_ONE_MINUS_SRC_ALPHA).
		let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some(label),
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
						color: wgpu::BlendComponent {
							src_factor: wgpu::BlendFactor::SrcAlpha,
							dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
							operation: wgpu::BlendOperation::Add,
						},
						alpha: wgpu::BlendComponent {
							src_factor: wgpu::BlendFactor::One,
							dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
							operation: wgpu::BlendOperation::Add,
						},
					}),
					write_mask: wgpu::ColorWrites::ALL,
				})],
			}),
			multiview_mask: None,
			cache: None,
		});
		let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
			label: Some(label),
			size: UNIFORM_SIZE,
			usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
			mapped_at_creation: false,
		});
		let sampler = |mode| {
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
		WorldPass {
			label,
			pipeline,
			layout,
			uniforms,
			repeat: sampler(wgpu::AddressMode::Repeat),
			clamp: sampler(wgpu::AddressMode::ClampToEdge),
		}
	}

	/// A draw of the pass, for a pass of a frame's encoder to record (see
	/// [`PassDraw::record`]): its uniforms written, its textures bound (`texture`
	/// repeats across the map; `mask`, for a masked pass, is clamped to the map's
	/// edges). One a frame: the uniforms are the pass's own.
	pub fn prepare(&self, gpu: &Gpu, params: &PassParams, texture: &GpuTexture, mask: Option<&wgpu::TextureView>) -> PassDraw {
		let view = params.view;
		let floats: [f32; 16] = [
			view.center[0],
			view.center[1],
			view.width as f32 / view.zoom / 2.0,
			view.height as f32 / view.zoom / 2.0,
			params.map_extent.0,
			params.map_extent.1,
			params.texture_size.0,
			params.texture_size.1,
			params.factor,
			0.0,
			0.0,
			0.0,
			params.ambient[0],
			params.ambient[1],
			params.ambient[2],
			params.ambient[3],
		];
		let bytes: Vec<u8> = floats.iter().flat_map(|f| f.to_le_bytes()).collect();
		gpu.queue.write_buffer(&self.uniforms, 0, &bytes);
		let mut entries = vec![
			wgpu::BindGroupEntry {
				binding: 0,
				resource: self.uniforms.as_entire_binding(),
			},
			wgpu::BindGroupEntry {
				binding: 1,
				resource: wgpu::BindingResource::TextureView(&texture.view),
			},
			wgpu::BindGroupEntry {
				binding: 2,
				resource: wgpu::BindingResource::Sampler(&self.repeat),
			},
		];
		if let Some(mask) = mask {
			entries.extend([
				wgpu::BindGroupEntry {
					binding: 3,
					resource: wgpu::BindingResource::TextureView(mask),
				},
				wgpu::BindGroupEntry {
					binding: 4,
					resource: wgpu::BindingResource::Sampler(&self.clamp),
				},
			]);
		}
		let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some(self.label),
			layout: &self.layout,
			entries: &entries,
		});
		PassDraw {
			pipeline: self.pipeline.clone(),
			group,
		}
	}
}

/// A pass's draw made ready (see [`WorldPass::prepare`]).
pub(crate) struct PassDraw {
	pipeline: wgpu::RenderPipeline,
	group: wgpu::BindGroup,
}

impl PassDraw {
	/// Record it into `pass`.
	pub(crate) fn record(&self, pass: &mut wgpu::RenderPass) {
		pass.set_pipeline(&self.pipeline);
		pass.set_bind_group(0, &self.group, &[]);
		pass.draw(0..3, 0..1);
	}
}
