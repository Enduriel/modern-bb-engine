//! [`RenderTarget`]: an offscreen colour target that can be read back as an image.

use std::sync::mpsc::channel;

use bb_assets::Image;

use crate::gpu::Gpu;

/// The colour format of offscreen targets.
pub const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// An RGBA8 texture to render into and read back.
pub struct RenderTarget {
	pub texture: wgpu::Texture,
	pub view: wgpu::TextureView,
	pub width: u32,
	pub height: u32,
}

impl RenderTarget {
	pub fn new(gpu: &Gpu, width: u32, height: u32) -> Self {
		let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
			label: Some("render target"),
			size: wgpu::Extent3d {
				width,
				height,
				depth_or_array_layers: 1,
			},
			mip_level_count: 1,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: TARGET_FORMAT,
			usage: wgpu::TextureUsages::RENDER_ATTACHMENT
				| wgpu::TextureUsages::COPY_SRC
				| wgpu::TextureUsages::TEXTURE_BINDING,
			view_formats: &[],
		});
		let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
		RenderTarget {
			texture,
			view,
			width,
			height,
		}
	}

	/// Fill the target with opaque black, as a frame with nothing in it.
	pub fn clear(&self, gpu: &Gpu) {
		let mut encoder = gpu
			.device
			.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
		encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
			label: Some("clear"),
			color_attachments: &[Some(wgpu::RenderPassColorAttachment {
				view: &self.view,
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
		gpu.queue.submit([encoder.finish()]);
	}

	/// Copy the target's pixels back (rows top to bottom).
	pub fn read(&self, gpu: &Gpu) -> Result<Image, String> {
		let row = self.width * 4;
		let padded_row =
			row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
		let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
			label: Some("readback"),
			size: u64::from(padded_row) * u64::from(self.height),
			usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
			mapped_at_creation: false,
		});
		let mut encoder = gpu
			.device
			.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
		encoder.copy_texture_to_buffer(
			self.texture.as_image_copy(),
			wgpu::TexelCopyBufferInfo {
				buffer: &buffer,
				layout: wgpu::TexelCopyBufferLayout {
					offset: 0,
					bytes_per_row: Some(padded_row),
					rows_per_image: Some(self.height),
				},
			},
			wgpu::Extent3d {
				width: self.width,
				height: self.height,
				depth_or_array_layers: 1,
			},
		);
		gpu.queue.submit([encoder.finish()]);

		let (sender, receiver) = channel();
		buffer.map_async(wgpu::MapMode::Read, .., move |result| {
			let _ = sender.send(result);
		});
		gpu.wait()?;
		receiver
			.recv()
			.map_err(|e| e.to_string())?
			.map_err(|e| e.to_string())?;
		let mapped = buffer.get_mapped_range(..).map_err(|e| e.to_string())?;
		let mut rgba = Vec::with_capacity((row * self.height) as usize);
		for line in mapped.chunks(padded_row as usize) {
			rgba.extend_from_slice(&line[..row as usize]);
		}
		Ok(Image {
			width: self.width,
			height: self.height,
			rgba,
		})
	}
}
