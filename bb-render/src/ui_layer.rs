//! [`UiLayer`]: the UI page drawn over the game's frame.
//!
//! The page renders offscreen in the browser host (`bb-ui`), on the GPU: its paints are
//! copied into the [`UiPage`]'s texture (premultiplied BGRA) as they happen. It's laid
//! out at the screen size divided by the UI scale and painted at the screen's
//! resolution (the scale is its device scale factor), as the original zooms it.
//! So it's drawn a texel a pixel, from the frame's top-left, and blended over
//! what the game drew, before the frame is presented.

use crate::gpu::Gpu;
use crate::target::{RenderTarget, TARGET_FORMAT};
use crate::ui_page::UiPage;

const SHADER: &str = r"
@group(0) @binding(0) var page: texture_2d<f32>;

struct Out {
	@builtin(position) position: vec4<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> Out {
	// One triangle over the frame: (-1, -1), (3, -1), (-1, 3).
	let xy = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
	var out: Out;
	out.position = vec4<f32>(xy * 2.0 - 1.0, 0.0, 1.0);
	return out;
}

@fragment
fn fs_main(f: Out) -> @location(0) vec4<f32> {
	// A texel a pixel; nothing past the page (a pixel short of the frame, when the
	// scale doesn't divide the screen).
	let at = vec2<u32>(f.position.xy);
	let size = textureDimensions(page);
	if (at.x >= size.x || at.y >= size.y) {
		return vec4<f32>(0.0);
	}
	return textureLoad(page, at, 0);
}
";

/// The program that blends the page's latest frame over a frame.
pub struct UiLayer {
	pipeline: wgpu::RenderPipeline,
	layout: wgpu::BindGroupLayout,
	/// The page's frame, which the browser's paints are copied into.
	page: UiPage,
	/// The bind group of the page's texture, and that texture's generation.
	group: Option<(u64, wgpu::BindGroup)>,
}

impl UiLayer {
	/// The bytes the page's texture takes on the GPU (none before its first paint).
	pub fn bytes(&self) -> u64 {
		self.page.bytes()
	}

	/// The page this layer draws.
	pub fn page(&self) -> &UiPage {
		&self.page
	}
}

impl UiLayer {
	/// The layer drawing `page`, on `gpu` (the page's).
	pub fn new(gpu: &Gpu, page: UiPage) -> Self {
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
						sample_type: wgpu::TextureSampleType::Float { filterable: false },
						view_dimension: wgpu::TextureViewDimension::D2,
						multisampled: false,
					},
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
		UiLayer {
			pipeline,
			layout,
			page,
			group: None,
		}
	}

	/// Blend the page over `target`, a texel a pixel from its top-left, as last painted.
	/// Nothing before the first paint.
	pub fn draw(&mut self, gpu: &Gpu, target: &RenderTarget) {
		let Some((view, generation)) = self.page.current() else { return };
		if self.group.as_ref().is_none_or(|(g, _)| *g != generation) {
			let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
				label: Some("ui layer"),
				layout: &self.layout,
				entries: &[wgpu::BindGroupEntry {
					binding: 0,
					resource: wgpu::BindingResource::TextureView(&view),
				}],
			});
			self.group = Some((generation, group));
		}
		let Some((_, group)) = &self.group else { return };
		let mut encoder = gpu
			.device
			.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("ui layer") });
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
			pass.set_bind_group(0, group, &[]);
			pass.draw(0..3, 0..1);
		}
		gpu.submit("gpu ui layer", encoder.finish());
	}
}
