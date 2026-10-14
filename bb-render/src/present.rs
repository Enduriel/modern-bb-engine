//! [`Presenter`]: a finished frame onto a window. The frame is an offscreen
//! [`RenderTarget`] the size of the window; one screen-covering triangle copies it onto
//! the surface's texture, whatever the surface's format. The window is opaque: the
//! frame's alpha (what blending left in it) is not shown through.

use crate::gpu::Gpu;
use crate::target::RenderTarget;

const SHADER: &str = r"
@group(0) @binding(0) var frame: texture_2d<f32>;
@group(0) @binding(1) var smp: sampler;

struct Out {
	@builtin(position) position: vec4<f32>,
	@location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> Out {
	// One triangle over the screen: (-1, -1), (3, -1), (-1, 3).
	let xy = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
	var out: Out;
	out.position = vec4<f32>(xy * 2.0 - 1.0, 0.0, 1.0);
	out.uv = vec2<f32>(xy.x, 1.0 - xy.y);
	return out;
}

@fragment
fn fs_main(f: Out) -> @location(0) vec4<f32> {
	return vec4<f32>(textureSample(frame, smp, f.uv).rgb, 1.0);
}
";

/// The copy program for one surface format.
pub struct Presenter {
	pipeline: wgpu::RenderPipeline,
	layout: wgpu::BindGroupLayout,
	sampler: wgpu::Sampler,
}

impl Presenter {
	pub fn new(gpu: &Gpu, format: wgpu::TextureFormat) -> Self {
		let device = &gpu.device;
		let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
			label: Some("present"),
			source: wgpu::ShaderSource::Wgsl(SHADER.into()),
		});
		let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("present"),
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
			label: Some("present"),
			bind_group_layouts: &[Some(&layout)],
			immediate_size: 0,
		});
		let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("present"),
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
					format,
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
		// The frame is the window's size, so nearest texels copy it exactly.
		let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
			label: Some("present"),
			..Default::default()
		});
		Presenter {
			pipeline,
			layout,
			sampler,
		}
	}

	/// Copy `frame` onto `surface_view`.
	pub fn draw(&self, gpu: &Gpu, frame: &RenderTarget, surface_view: &wgpu::TextureView) {
		let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some("present"),
			layout: &self.layout,
			entries: &[
				wgpu::BindGroupEntry {
					binding: 0,
					resource: wgpu::BindingResource::TextureView(&frame.view),
				},
				wgpu::BindGroupEntry {
					binding: 1,
					resource: wgpu::BindingResource::Sampler(&self.sampler),
				},
			],
		});
		let mut encoder = gpu
			.device
			.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
		{
			let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
				label: Some("present"),
				color_attachments: &[Some(wgpu::RenderPassColorAttachment {
					view: surface_view,
					depth_slice: None,
					resolve_target: None,
					ops: wgpu::Operations {
						load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
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
