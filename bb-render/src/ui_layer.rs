//! [`UiLayer`]: the UI page drawn over the game's frame.
//!
//! The page renders offscreen in the browser host (`bb-ui`), which hands over each
//! painted frame as premultiplied BGRA. It's laid out at the screen size divided by
//! the UI scale and painted at the screen's resolution (the scale is its device scale
//! factor), as the original zooms it. So it's drawn a texel a pixel, from
//! the frame's top-left, and blended over what the game drew, before the frame is
//! presented.
//!
//! Only what the page repainted is uploaded ([`UiLayer::upload_regions`]): a hover or a
//! small animation is kilobytes a frame, not the whole page (14 MiB at 2560x1358). The
//! bytes go through staging memory kept from upload to upload; a full-page change keeps
//! a chunk of the page's size (`page_paint` in bb-app measures it all).

use crate::gpu::Gpu;
use crate::target::{RenderTarget, TARGET_FORMAT};

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

/// The staging belt's chunk: big enough for most paints' changes in one; a bigger one
/// (a whole 2560x1440 frame is 14 MiB) gets a chunk of its size, reused after.
const STAGING_CHUNK: u64 = 4 << 20;

/// The page's latest frame on the GPU, and the program that blends it over a frame.
pub struct UiLayer {
	pipeline: wgpu::RenderPipeline,
	layout: wgpu::BindGroupLayout,
	/// The page's texture and its bind group, once a frame was uploaded.
	page: Option<Page>,
	/// Mapped staging memory the uploads are written through, reused from upload to
	/// upload (`Queue::write_texture` makes fresh staging memory for every write, and
	/// touching megabytes of new pages for a full-frame paint cost milliseconds).
	belt: wgpu::util::StagingBelt,
	/// The copies of the uploads since the last draw, which submits them with it.
	copies: Option<wgpu::CommandEncoder>,
	uploads: UploadStats,
}

/// A part of the page's frame, in its pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
	pub x: u32,
	pub y: u32,
	pub width: u32,
	pub height: u32,
}

/// What uploading the page's paints has cost so far.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UploadStats {
	/// Uploads that wrote something (each in one or more regions).
	pub uploads: u64,
	/// Regions written to the texture.
	pub regions: u64,
	/// Bytes handed to the GPU queue.
	pub bytes: u64,
	/// Time spent handing them over (copying them into staging memory).
	pub time: std::time::Duration,
}

impl UploadStats {
	/// What happened between `earlier` and these.
	pub fn since(&self, earlier: &UploadStats) -> UploadStats {
		UploadStats {
			uploads: self.uploads - earlier.uploads,
			regions: self.regions - earlier.regions,
			bytes: self.bytes - earlier.bytes,
			time: self.time.saturating_sub(earlier.time),
		}
	}
}

impl UiLayer {
	/// The bytes the page's texture takes on the GPU (none before its first frame).
	pub fn bytes(&self) -> u64 {
		self.page.as_ref().map_or(0, |page| crate::texture::texture_bytes(&page.texture))
	}

	/// What uploading the page's paints has cost so far.
	pub fn upload_stats(&self) -> UploadStats {
		self.uploads
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
			page: None,
			belt: wgpu::util::StagingBelt::new(gpu.device.clone(), STAGING_CHUNK),
			copies: None,
			uploads: UploadStats::default(),
		}
	}

	/// Take the page's frame: `width` × `height` premultiplied BGRA pixels, rows top to
	/// bottom.
	pub fn upload(&mut self, gpu: &Gpu, width: u32, height: u32, bgra: &[u8]) {
		self.upload_regions(gpu, width, height, bgra, &[Region { x: 0, y: 0, width, height }]);
	}

	/// Take the parts `regions` of the page's frame that changed since the last upload:
	/// `bgra` is the whole frame, as for [`upload`](Self::upload). Only those parts are
	/// copied to the GPU, unless the frame's size changed: then all of it is.
	pub fn upload_regions(&mut self, gpu: &Gpu, width: u32, height: u32, bgra: &[u8], regions: &[Region]) {
		if width == 0 || height == 0 || bgra.len() != (width * height * 4) as usize {
			return;
		}
		let whole = [Region { x: 0, y: 0, width, height }];
		let regions = if matches!(&self.page, Some(p) if (p.width, p.height) == (width, height)) {
			regions
		} else {
			self.page = Some(self.new_page(gpu, width, height));
			&whole[..]
		};
		let Some(page) = &self.page else { return };
		let started = std::time::Instant::now();
		let encoder = self.copies.get_or_insert_with(|| {
			gpu.device
				.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("ui layer") })
		});
		let mut wrote = false;
		let row = width as usize * 4;
		for region in regions {
			// Inside the frame (a caller's mistake mustn't fail the frame's validation).
			let (x, y) = (region.x.min(width), region.y.min(height));
			let (w, h) = (region.width.min(width - x), region.height.min(height - y));
			if w == 0 || h == 0 {
				continue;
			}
			let bytes = w as usize * 4;
			let stride = bytes.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize);
			let size = wgpu::BufferSize::new((stride * h as usize) as u64).expect("not empty");
			let alignment = wgpu::BufferSize::new(u64::from(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)).expect("not zero");
			let slice = self.belt.allocate(size, alignment);
			{
				let mut staging = slice.get_mapped_range_mut().expect("the belt hands out mapped memory");
				let first = y as usize * row + x as usize * 4;
				for i in 0..h as usize {
					let at = first + i * row;
					staging.slice(i * stride..i * stride + bytes).copy_from_slice(&bgra[at..at + bytes]);
				}
			}
			encoder.copy_buffer_to_texture(
				wgpu::TexelCopyBufferInfo {
					buffer: slice.buffer(),
					layout: wgpu::TexelCopyBufferLayout {
						offset: slice.offset(),
						bytes_per_row: Some(stride as u32),
						rows_per_image: Some(h),
					},
				},
				wgpu::TexelCopyTextureInfo {
					texture: &page.texture,
					mip_level: 0,
					origin: wgpu::Origin3d { x, y, z: 0 },
					aspect: wgpu::TextureAspect::All,
				},
				wgpu::Extent3d {
					width: w,
					height: h,
					depth_or_array_layers: 1,
				},
			);
			wrote = true;
			self.uploads.regions += 1;
			self.uploads.bytes += (bytes * h as usize) as u64;
		}
		if wrote {
			self.uploads.uploads += 1;
			self.uploads.time += started.elapsed();
		}
	}

	fn new_page(&self, gpu: &Gpu, width: u32, height: u32) -> Page {
		let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
			label: Some("ui page"),
			size: wgpu::Extent3d {
				width,
				height,
				depth_or_array_layers: 1,
			},
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
			entries: &[wgpu::BindGroupEntry {
				binding: 0,
				resource: wgpu::BindingResource::TextureView(&view),
			}],
		});
		Page {
			texture,
			group,
			width,
			height,
		}
	}

	/// Blend the page over `target`, a texel a pixel from its top-left, with what was
	/// uploaded since the last draw. Nothing before the first upload.
	pub fn draw(&mut self, gpu: &Gpu, target: &RenderTarget) {
		let Some(page) = &self.page else { return };
		let copied = self.copies.is_some();
		let mut encoder = self.copies.take().unwrap_or_else(|| {
			gpu.device
				.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("ui layer") })
		});
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
		if copied {
			self.belt.finish();
		}
		gpu.queue.submit([encoder.finish()]);
		if copied {
			self.belt.recall();
		}
	}
}
