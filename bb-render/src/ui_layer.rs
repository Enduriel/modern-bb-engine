//! [`UiLayer`]: the UI page drawn over the game's frame.
//!
//! The page renders offscreen in the browser host (`bb-ui`), which hands over each
//! painted frame as premultiplied BGRA. It's laid out at the screen size divided by
//! the UI scale, so it's stretched over the whole frame, as the original stretched it,
//! and blended over what the game drew, before the frame is presented.

use crate::gpu::Gpu;
use crate::target::{RenderTarget, TARGET_FORMAT};

const SHADER: &str = r"
@group(0) @binding(0) var page: texture_2d<f32>;
@group(0) @binding(1) var smp: sampler;

struct Out {
	@builtin(position) position: vec4<f32>,
	@location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> Out {
	// One triangle over the frame: (-1, -1), (3, -1), (-1, 3).
	let xy = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
	var out: Out;
	out.position = vec4<f32>(xy * 2.0 - 1.0, 0.0, 1.0);
	out.uv = vec2<f32>(xy.x, 1.0 - xy.y);
	return out;
}

@fragment
fn fs_main(f: Out) -> @location(0) vec4<f32> {
	return textureSample(page, smp, f.uv);
}
";

/// The page's latest frame on the GPU, and the program that blends it over a frame.
pub struct UiLayer {
	pipeline: wgpu::RenderPipeline,
	layout: wgpu::BindGroupLayout,
	sampler: wgpu::Sampler,
	/// The page's texture and its bind group, once a frame was uploaded.
	page: Option<Page>,
}

impl UiLayer {
	/// The bytes the page's texture takes on the GPU (none before its first frame).
	pub fn bytes(&self) -> u64 {
		self.page.as_ref().map_or(0, |page| crate::texture::texture_bytes(&page.texture))
	}
}

struct Page {
	texture: wgpu::Texture,
	group: wgpu::BindGroup,
	width: u32,
	height: u32,
}

impl UiLayer {
	pub fn new(gpu: &Gpu) -> Self {
		let device = &gpu.device;
		let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
			label: Some("ui layer"),
			source: wgpu::ShaderSource::Wgsl(SHADER.into()),
		});
		let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("ui layer"),
			entries: &[
				wgpu::BindGroupLayoutEntry {
					binding: 0,
					visibility: wgpu::ShaderStages::FRAGMENT,
					ty: wgpu::BindingType::Texture {
						sample_type: wgpu::TextureSampleType::Float { filterable: true },
						view_dimension: wgpu::TextureViewDimension::D2,
						multisampled: false,
					},
					count: None,
				},
				wgpu::BindGroupLayoutEntry {
					binding: 1,
					visibility: wgpu::ShaderStages::FRAGMENT,
					ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
					count: None,
				},
			],
		});
		let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
			label: Some("ui layer"),
			bind_group_layouts: &[Some(&layout)],
			immediate_size: 0,
		});
		// The page's colours are premultiplied by its alpha.
		let premultiplied = wgpu::BlendComponent {
			src_factor: wgpu::BlendFactor::One,
			dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
			operation: wgpu::BlendOperation::Add,
		};
		let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("ui layer"),
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
					format: TARGET_FORMAT,
					blend: Some(wgpu::BlendState {
						color: premultiplied,
						alpha: premultiplied,
					}),
					write_mask: wgpu::ColorWrites::ALL,
				})],
			}),
			primitive: wgpu::PrimitiveState::default(),
			depth_stencil: None,
			multisample: wgpu::MultisampleState::default(),
			multiview_mask: None,
			cache: None,
		});
		// A page smaller than the frame (UI scale above 1) is stretched smoothly.
		let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
			label: Some("ui layer"),
			mag_filter: wgpu::FilterMode::Linear,
			min_filter: wgpu::FilterMode::Linear,
			..Default::default()
		});
		UiLayer {
			pipeline,
			layout,
			sampler,
			page: None,
		}
	}

	/// Take the page's frame: `width` × `height` premultiplied BGRA pixels, rows top to
	/// bottom.
	pub fn upload(&mut self, gpu: &Gpu, width: u32, height: u32, bgra: &[u8]) {
		if width == 0 || height == 0 || bgra.len() != (width * height * 4) as usize {
			return;
		}
		let size = wgpu::Extent3d {
			width,
			height,
			depth_or_array_layers: 1,
		};
		if !matches!(&self.page, Some(p) if (p.width, p.height) == (width, height)) {
			let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
				label: Some("ui page"),
				size,
				mip_level_count: 1,
				sample_count: 1,
				dimension: wgpu::TextureDimension::D2,
				format: wgpu::TextureFormat::Bgra8Unorm,
				usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
				view_formats: &[],
			});
			let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
			let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
				label: Some("ui layer"),
				layout: &self.layout,
				entries: &[
					wgpu::BindGroupEntry {
						binding: 0,
						resource: wgpu::BindingResource::TextureView(&view),
					},
					wgpu::BindGroupEntry {
						binding: 1,
						resource: wgpu::BindingResource::Sampler(&self.sampler),
					},
				],
			});
			self.page = Some(Page {
				texture,
				group,
				width,
				height,
			});
		}
		let Some(page) = &self.page else { return };
		gpu.queue.write_texture(
			page.texture.as_image_copy(),
			bgra,
			wgpu::TexelCopyBufferLayout {
				offset: 0,
				bytes_per_row: Some(width * 4),
				rows_per_image: Some(height),
			},
			size,
		);
	}

	/// Blend the page over `target`, stretched to cover it. Nothing before the first
	/// upload.
	pub fn draw(&self, gpu: &Gpu, target: &RenderTarget) {
		let Some(page) = &self.page else { return };
		let mut encoder = gpu
			.device
			.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
		{
			let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
				label: Some("ui layer"),
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
			pass.set_bind_group(0, &page.group, &[]);
			pass.draw(0..3, 0..1);
		}
		gpu.queue.submit([encoder.finish()]);
	}
}
