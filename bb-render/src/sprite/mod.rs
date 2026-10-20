//! Drawing sprites: textured quads in world units through the original's sprite
//! program, batched up to eight atlases per draw call.
//!
//! A frame is a list of [`Quad`]s in draw order. Consecutive quads share a draw call
//! while they need at most eight distinct atlases between them; the ninth starts a
//! new call. That is the limit the original's shader is built for
//! (`NUM_TEXTURE_SAMPLERS`, eight `if` branches in `sprite.fs`).

use std::num::NonZeroU64;
use std::ops::Range;

use crate::gpu::Gpu;
use crate::target::{RenderTarget, TARGET_FORMAT};
use crate::texture::GpuTexture;

pub(crate) mod instances;

/// Atlases one draw call can sample.
pub const SLOTS: usize = 8;

/// Bytes per vertex: position (4 × f32), two colours (4 × u8 each), texture
/// coordinates (2 × f32), atlas slot (u32), saturation and factor (f32), flags (u32),
/// level (u32).
const VERTEX_SIZE: u64 = 52;

/// Bytes of the uniform block.
const UNIFORM_SIZE: u64 = 64;

/// The shader's uniforms: the camera and the scene's ambient light.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneUniforms {
	/// Subtracted from every position (the camera's world position).
	pub translation: [f32; 2],
	/// Multiplies the translated position into clip space.
	pub scale: [f32; 3],
	/// Multiplies every sprite drawn with `flags == 0`.
	pub ambient_color: [f32; 4],
	/// Multiplies each sprite's saturation.
	pub ambient_saturation: f32,
	/// The battle camera's level and the depth of field's strength: a sprite `n` levels
	/// below the camera blends `strength × n` of its mip level 2 in (the original's
	/// `sprite_dof` program, `DOF_STRENGTH` 0.25). Strength 0 draws everything sharp,
	/// as the world's `sprite` program does.
	pub camera_level: f32,
	pub dof_strength: f32,
}

impl SceneUniforms {
	/// A camera centred on `center` showing `width` × `height` world units, y up,
	/// with neutral ambient light.
	pub fn centered(center: [f32; 2], width: f32, height: f32) -> Self {
		SceneUniforms {
			translation: center,
			scale: [2.0 / width, 2.0 / height, 1.0],
			ambient_color: [1.0; 4],
			ambient_saturation: 1.0,
			camera_level: 0.0,
			dof_strength: 0.0,
		}
	}

	fn bytes(&self) -> [u8; UNIFORM_SIZE as usize] {
		let mut out = [0u8; UNIFORM_SIZE as usize];
		let floats = [
			self.translation[0],
			self.translation[1],
			0.0,
			0.0,
			self.scale[0],
			self.scale[1],
			self.scale[2],
			self.ambient_saturation,
			self.ambient_color[0],
			self.ambient_color[1],
			self.ambient_color[2],
			self.ambient_color[3],
			self.camera_level,
			self.dof_strength,
			0.0,
			0.0,
		];
		for (chunk, value) in out.chunks_exact_mut(4).zip(floats) {
			chunk.copy_from_slice(&value.to_le_bytes());
		}
		out
	}
}

/// One textured quad, in world units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quad {
	/// Which atlas, as an index into the texture list passed to [`SpriteRenderer::draw`].
	pub texture: usize,
	/// Corner positions, in the order of `uv`.
	pub corners: [[f32; 2]; 4],
	/// Texture coordinates of the corners.
	pub uv: [[f32; 2]; 4],
	/// Multiplies the texel (the sprite's colour and alpha).
	pub color: [u8; 4],
	/// Per-corner colours in place of `color`, for a gradient (a battle tile's wall
	/// darkens towards its foot).
	pub corner_colors: Option<[[u8; 4]; 4]>,
	/// Blended over the result by `factor` (the shaker's flash colour).
	pub overlay: [u8; 4],
	pub factor: f32,
	pub saturation: f32,
	/// 0 = lit by the ambient colour, anything else = unlit.
	pub flags: u32,
	/// The height level it's drawn at, for the depth of field.
	pub level: u32,
}

/// The sprite program and its fixed GPU objects.
pub struct SpriteRenderer {
	pipeline: wgpu::RenderPipeline,
	/// The same program drawing quads kept as instances (see [`instances`]).
	instanced: wgpu::RenderPipeline,
	instances_layout: wgpu::BindGroupLayout,
	uniforms: wgpu::Buffer,
	uniform_group: wgpu::BindGroup,
	textures_layout: wgpu::BindGroupLayout,
	sampler: wgpu::Sampler,
	/// Bound to slots a draw call doesn't use.
	blank: GpuTexture,
	streams: std::cell::RefCell<Streams>,
}

impl SpriteRenderer {
	/// The bytes of its own on the GPU: its textures and the kept quads' buffers.
	pub fn bytes(&self) -> u64 {
		let streams = self.streams.borrow();
		self.blank.bytes() + streams.kept.buffer.as_ref().map_or(0, wgpu::Buffer::size) + streams.instances.bytes()
	}

	pub fn new(gpu: &Gpu) -> Self {
		let device = &gpu.device;
		let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
			label: Some("sprite"),
			source: wgpu::ShaderSource::Wgsl(include_str!("sprite.wgsl").into()),
		});
		let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("sprite uniforms"),
			entries: &[wgpu::BindGroupLayoutEntry {
				binding: 0,
				visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
				ty: wgpu::BindingType::Buffer {
					ty: wgpu::BufferBindingType::Uniform,
					has_dynamic_offset: false,
					min_binding_size: NonZeroU64::new(UNIFORM_SIZE),
				},
				count: None,
			}],
		});
		let mut texture_entries = Vec::new();
		for slot in 0..SLOTS as u32 {
			texture_entries.push(wgpu::BindGroupLayoutEntry {
				binding: slot,
				visibility: wgpu::ShaderStages::FRAGMENT,
				ty: wgpu::BindingType::Texture {
					sample_type: wgpu::TextureSampleType::Float { filterable: true },
					view_dimension: wgpu::TextureViewDimension::D2,
					multisampled: false,
				},
				count: None,
			});
		}
		for slot in 0..SLOTS as u32 {
			texture_entries.push(wgpu::BindGroupLayoutEntry {
				binding: SLOTS as u32 + slot,
				visibility: wgpu::ShaderStages::FRAGMENT,
				ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
				count: None,
			});
		}
		let textures_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("sprite atlases"),
			entries: &texture_entries,
		});
		let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
			label: Some("sprite"),
			bind_group_layouts: &[Some(&uniform_layout), Some(&textures_layout)],
			immediate_size: 0,
		});
		let attributes = wgpu::vertex_attr_array![
			0 => Float32x4,
			1 => Unorm8x4,
			2 => Unorm8x4,
			3 => Float32x2,
			4 => Uint32,
			5 => Float32,
			6 => Float32,
			7 => Uint32,
			8 => Uint32,
		];
		// The original's sprite blend mode (1 of its switch at 0x52fd40):
		// glBlendFuncSeparate(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA, GL_ONE,
		// GL_ONE_MINUS_SRC_ALPHA). Only images read back see the alpha: a portrait
		// drawn onto transparency keeps each texel's alpha, its colour premultiplied.
		let blend = wgpu::BlendComponent {
			src_factor: wgpu::BlendFactor::SrcAlpha,
			dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
			operation: wgpu::BlendOperation::Add,
		};
		let blend_alpha = wgpu::BlendComponent {
			src_factor: wgpu::BlendFactor::One,
			dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
			operation: wgpu::BlendOperation::Add,
		};
		let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("sprite"),
			layout: Some(&layout),
			vertex: wgpu::VertexState {
				module: &shader,
				entry_point: Some("vs_main"),
				compilation_options: Default::default(),
				buffers: &[Some(wgpu::VertexBufferLayout {
					array_stride: VERTEX_SIZE,
					step_mode: wgpu::VertexStepMode::Vertex,
					attributes: &attributes,
				})],
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
						alpha: blend_alpha,
					}),
					write_mask: wgpu::ColorWrites::ALL,
				})],
			}),
			multiview_mask: None,
			cache: None,
		});
		let instances_layout = instances::layout(gpu);
		let instanced_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
			label: Some("sprite instanced"),
			source: wgpu::ShaderSource::Wgsl(concat!(include_str!("sprite.wgsl"), include_str!("instanced.wgsl")).into()),
		});
		let instanced_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
			label: Some("sprite instanced"),
			bind_group_layouts: &[Some(&uniform_layout), Some(&textures_layout), Some(&instances_layout)],
			immediate_size: 0,
		});
		let instanced = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("sprite instanced"),
			layout: Some(&instanced_layout),
			vertex: wgpu::VertexState {
				module: &instanced_shader,
				entry_point: Some("vs_instanced"),
				compilation_options: Default::default(),
				buffers: &[],
			},
			primitive: wgpu::PrimitiveState::default(),
			depth_stencil: None,
			multisample: wgpu::MultisampleState::default(),
			fragment: Some(wgpu::FragmentState {
				module: &instanced_shader,
				entry_point: Some("fs_main"),
				compilation_options: Default::default(),
				targets: &[Some(wgpu::ColorTargetState {
					format: TARGET_FORMAT,
					blend: Some(wgpu::BlendState {
						color: blend,
						alpha: blend_alpha,
					}),
					write_mask: wgpu::ColorWrites::ALL,
				})],
			}),
			multiview_mask: None,
			cache: None,
		});
		let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
			label: Some("sprite uniforms"),
			size: UNIFORM_SIZE,
			usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
			mapped_at_creation: false,
		});
		let uniform_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some("sprite uniforms"),
			layout: &uniform_layout,
			entries: &[wgpu::BindGroupEntry {
				binding: 0,
				resource: uniforms.as_entire_binding(),
			}],
		});
		let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
			label: Some("atlas"),
			address_mode_u: wgpu::AddressMode::ClampToEdge,
			address_mode_v: wgpu::AddressMode::ClampToEdge,
			mag_filter: wgpu::FilterMode::Linear,
			min_filter: wgpu::FilterMode::Linear,
			mipmap_filter: wgpu::MipmapFilterMode::Linear,
			..Default::default()
		});
		let blank_image = bb_assets::Image {
			width: 1,
			height: 1,
			rgba: vec![0; 4],
		};
		let blank = GpuTexture::upload(gpu, "blank", &blank_image);
		SpriteRenderer {
			pipeline,
			instanced,
			instances_layout,
			uniforms,
			uniform_group,
			textures_layout,
			sampler,
			blank,
			streams: std::cell::RefCell::new(Streams::new(gpu)),
		}
	}

	/// Draw `quads` in order into `target`, after clearing it to `clear` (if given).
	/// Each quad's `texture` indexes `textures`.
	pub fn draw(
		&self,
		gpu: &Gpu,
		target: &RenderTarget,
		scene: &SceneUniforms,
		clear: Option<[f64; 4]>,
		quads: &[Quad],
		textures: &[&GpuTexture],
	) {
		self.draw_parts(gpu, target, scene, clear, &[Part::Streamed(quads)], textures);
	}

	/// Draw `parts` in order into `target`, after clearing it to `clear` (if given): quads
	/// streamed now (each one's `texture` indexing `textures`), calls over the kept quads
	/// (see [`SpriteRenderer::keep`]) and over the kept instances (see
	/// [`SpriteRenderer::write_instance`]), in one pass.
	pub(crate) fn draw_parts(
		&self,
		gpu: &Gpu,
		target: &RenderTarget,
		scene: &SceneUniforms,
		clear: Option<[f64; 4]>,
		parts: &[Part],
		textures: &[&GpuTexture],
	) {
		let mut encoder = gpu
			.device
			.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
		let prepared = self.prepare(gpu, &mut encoder, scene, parts, textures);
		{
			let mut pass = begin_pass(&mut encoder, target, clear, "sprites");
			self.record(&mut pass, &prepared, parts, 0..parts.len());
		}
		self.finish(&encoder);
		gpu.queue.submit([encoder.finish()]);
	}

	/// Get `parts` ready to be recorded in a pass of `encoder` (see
	/// [`record`](Self::record)): the uniforms written, the streamed quads' vertices and
	/// the kept quads' and instances' writes recorded on `encoder`, and each call's bind
	/// group made. [`finish`](Self::finish) must follow before `encoder` is submitted.
	pub(crate) fn prepare(
		&self,
		gpu: &Gpu,
		encoder: &mut wgpu::CommandEncoder,
		scene: &SceneUniforms,
		parts: &[Part],
		textures: &[&GpuTexture],
	) -> Prepared {
		gpu.queue.write_buffer(&self.uniforms, 0, &scene.bytes());
		let mut streams = self.streams.borrow_mut();
		let streams = &mut *streams;
		// The streamed quads' vertices in order (quad `i`'s four at `4i`), each with its
		// call's slot for its atlas, built in memory of ours and copied for the upload in
		// one go (the upload's mapped memory is slow to write piecemeal).
		let mut streamed: Vec<Option<(usize, Vec<Call>)>> = Vec::new();
		let vertices = &mut streams.scratch;
		vertices.clear();
		let mut count = 0;
		for part in parts {
			match part {
				Part::Streamed(quads) => {
					let calls = batch(quads);
					for call in &calls {
						for quad in &quads[call.quads.clone()] {
							let slot = call.slots.iter().position(|&t| t == quad.texture).expect("batched") as u32;
							push_vertices(vertices, quad, slot);
						}
					}
					streamed.push(Some((count, calls)));
					count += quads.len();
				}
				_ => streamed.push(None),
			}
		}
		let bytes = vertices.len();
		let vertex_buffer = streams.vertices.write(gpu, encoder, vertices).clone();
		let kept = &mut streams.kept;
		if let Some(buffer) = &kept.buffer {
			for (at, range) in kept.writes.drain(..) {
				if let Some(size) = wgpu::BufferSize::new(range.len() as u64) {
					streams.vertices.belt.write_buffer(encoder, buffer, at, size).copy_from_slice(&kept.bytes[range]);
				}
			}
		}
		kept.bytes.clear();
		let instances = streams.instances.upload(gpu, &self.instances_layout, encoder, &mut streams.vertices.belt);
		let kept_quads = parts
			.iter()
			.filter_map(|p| match p {
				Part::Kept(calls) => calls.iter().map(|(r, _)| r.end as usize).max(),
				_ => None,
			})
			.max()
			.unwrap_or(0);
		let indices = streams.indices.quads(gpu, count.max(kept_quads)).clone();
		let streamed = streamed
			.into_iter()
			.map(|part| {
				part.map(|(base, calls)| {
					let calls = calls
						.into_iter()
						.map(|call| {
							let group = self.texture_group(gpu, &call.slots, textures);
							(call.quads, group)
						})
						.collect();
					(base, calls)
				})
			})
			.collect();
		Prepared {
			streamed,
			vertices: (vertex_buffer, bytes as u64),
			indices,
			kept: streams.kept.buffer.clone(),
			instances,
		}
	}

	/// Record `parts[range]` (as [`prepare`](Self::prepare)d) into `pass`.
	pub(crate) fn record(&self, pass: &mut wgpu::RenderPass, prepared: &Prepared, parts: &[Part], range: Range<usize>) {
		pass.set_bind_group(0, &self.uniform_group, &[]);
		// Which pipeline is set: none yet, the vertex buffers' or the instances'.
		let mut set = None;
		let plain = |pass: &mut wgpu::RenderPass, set: &mut Option<bool>| {
			if *set != Some(false) {
				pass.set_pipeline(&self.pipeline);
				pass.set_index_buffer(prepared.indices.slice(..), wgpu::IndexFormat::Uint32);
				*set = Some(false);
			}
		};
		for (i, part) in parts.iter().enumerate().filter(|(i, _)| range.contains(i)) {
			match part {
				Part::Streamed(_) => {
					let Some((base, calls)) = &prepared.streamed[i] else { continue };
					if calls.is_empty() {
						continue;
					}
					plain(pass, &mut set);
					let (buffer, bytes) = &prepared.vertices;
					pass.set_vertex_buffer(0, buffer.slice(..*bytes));
					for (quads, group) in calls {
						pass.set_bind_group(1, group, &[]);
						let (first, end) = ((base + quads.start) as u32, (base + quads.end) as u32);
						pass.draw_indexed(first * 6..end * 6, 0, 0..1);
					}
				}
				Part::Kept(calls) => {
					let Some(buffer) = &prepared.kept else { continue };
					if calls.is_empty() {
						continue;
					}
					plain(pass, &mut set);
					pass.set_vertex_buffer(0, buffer.slice(..));
					for (quads, group) in calls.iter() {
						pass.set_bind_group(1, *group, &[]);
						pass.draw_indexed(quads.start * 6..quads.end * 6, 0, 0..1);
					}
				}
				Part::Instanced(calls) => {
					let Some(instances) = &prepared.instances else { continue };
					if calls.is_empty() {
						continue;
					}
					if set != Some(true) {
						pass.set_pipeline(&self.instanced);
						pass.set_bind_group(2, instances, &[]);
						set = Some(true);
					}
					for (entries, group) in calls.iter() {
						pass.set_bind_group(1, *group, &[]);
						pass.draw(0..6, entries.clone());
					}
				}
			}
		}
	}

	/// The prepared draws' uploads are recorded: call before `encoder` is submitted.
	pub(crate) fn finish(&self, encoder: &wgpu::CommandEncoder) {
		self.streams.borrow_mut().vertices.belt.finish_and_recall_on_submit(encoder);
	}

	/// Keep room for `count` instances ([`Part::Instanced`]): whether the room was made
	/// anew, every instance to be written again.
	pub(crate) fn keep_instances(&self, gpu: &Gpu, count: usize) -> bool {
		self.streams.borrow_mut().instances.keep(gpu, count)
	}

	/// Write instance `at` as `quad` (in its atlas's or page's coordinates; its `texture`
	/// is ignored, the order list gives each its slot) with the next draw.
	pub(crate) fn write_instance(&self, at: usize, quad: &Quad) {
		self.streams.borrow_mut().instances.write(at, quad);
	}

	/// Draw the instances in the order `entries` gives from the next draw on (see
	/// [`instances::Instances::set_order`]).
	pub(crate) fn set_instance_order(&self, entries: &[u32]) {
		self.streams.borrow_mut().instances.set_order(entries);
	}

	/// Keep room for `quads` quads drawn from frame to frame ([`Part::Kept`]): whether
	/// the room was made anew, its quads to be written again (see
	/// [`SpriteRenderer::write_kept`]).
	pub(crate) fn keep(&self, gpu: &Gpu, quads: usize) -> bool {
		let kept = &mut self.streams.borrow_mut().kept;
		let size = (quads.max(1) as u64 * 4 * VERTEX_SIZE).next_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT);
		if kept.buffer.as_ref().is_some_and(|b| b.size() >= size) {
			return false;
		}
		kept.buffer = Some(gpu.device.create_buffer(&wgpu::BufferDescriptor {
			label: Some("kept sprite vertices"),
			size: size.next_power_of_two(),
			usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
			mapped_at_creation: false,
		}));
		kept.writes.clear();
		kept.bytes.clear();
		true
	}

	/// Write the kept quads from quad `first` on: `quads`, each `(quad, slot)` (its
	/// atlas's slot in the call drawing it; `None` draws nothing there). The write goes
	/// with the next draw.
	pub(crate) fn write_kept(&self, first: usize, quads: impl IntoIterator<Item = Option<(Quad, u32)>>) {
		let kept = &mut self.streams.borrow_mut().kept;
		let start = kept.bytes.len();
		for quad in quads {
			match quad {
				Some((quad, slot)) => push_vertices(&mut kept.bytes, &quad, slot),
				None => kept.bytes.extend_from_slice(&[0; 4 * VERTEX_SIZE as usize]),
			}
		}
		let end = kept.bytes.len();
		let at = first as u64 * 4 * VERTEX_SIZE;
		// A write right after the last one goes with it.
		match kept.writes.last_mut() {
			Some((last_at, range)) if *last_at + range.len() as u64 == at && range.end == start => range.end = end,
			_ => kept.writes.push((at, start..end)),
		}
	}

	/// The bind group sampling `views` in its slots, in order (blank past them).
	pub(crate) fn views_group(&self, gpu: &Gpu, views: &[&wgpu::TextureView]) -> wgpu::BindGroup {
		let mut entries: Vec<wgpu::BindGroupEntry> = (0..SLOTS)
			.map(|slot| wgpu::BindGroupEntry {
				binding: slot as u32,
				resource: wgpu::BindingResource::TextureView(views.get(slot).copied().unwrap_or(&self.blank.view)),
			})
			.collect();
		for slot in 0..SLOTS as u32 {
			entries.push(wgpu::BindGroupEntry {
				binding: SLOTS as u32 + slot,
				resource: wgpu::BindingResource::Sampler(&self.sampler),
			});
		}
		gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some("sprite atlases"),
			layout: &self.textures_layout,
			entries: &entries,
		})
	}

	/// The blank texture's view: bound for an atlas not ready, it draws nothing.
	pub(crate) fn blank_view(&self) -> &wgpu::TextureView {
		&self.blank.view
	}

	fn texture_group(
		&self,
		gpu: &Gpu,
		slots: &[usize],
		textures: &[&GpuTexture],
	) -> wgpu::BindGroup {
		let views: Vec<&wgpu::TextureView> = slots.iter().map(|&t| &textures[t].view).collect();
		self.views_group(gpu, &views)
	}
}

/// A part of a sprite pass (see [`SpriteRenderer::draw_parts`]).
pub(crate) enum Part<'a> {
	/// Quads streamed with this draw, batched in calls of up to [`SLOTS`] atlases.
	Streamed(&'a [Quad]),
	/// Calls over the kept quads: each a range of them and the atlases' bind group they
	/// were written for.
	Kept(&'a [(Range<u32>, &'a wgpu::BindGroup)]),
	/// Calls over the kept instances: each a range of the order list's entries and the
	/// atlases' bind group their slots index.
	Instanced(&'a [(Range<u32>, &'a wgpu::BindGroup)]),
}

/// Parts made ready for a pass (see [`SpriteRenderer::prepare`]).
pub(crate) struct Prepared {
	/// For each streamed part, its first quad in the vertex buffer and its calls: their
	/// quads (from the part's first) and bind groups.
	streamed: Vec<Option<(usize, Vec<StreamedCall>)>>,
	/// The streamed vertices' buffer and their bytes.
	vertices: (wgpu::Buffer, u64),
	indices: wgpu::Buffer,
	kept: Option<wgpu::Buffer>,
	instances: Option<wgpu::BindGroup>,
}

/// A streamed part's call: its quads (from the part's first) and bind group.
type StreamedCall = (Range<usize>, wgpu::BindGroup);

/// A pass over `target` of `encoder`, clearing it to `clear` first (if given).
pub(crate) fn begin_pass<'e>(
	encoder: &'e mut wgpu::CommandEncoder,
	target: &RenderTarget,
	clear: Option<[f64; 4]>,
	label: &str,
) -> wgpu::RenderPass<'e> {
	let load = match clear {
		Some([r, g, b, a]) => wgpu::LoadOp::Clear(wgpu::Color { r, g, b, a }),
		None => wgpu::LoadOp::Load,
	};
	encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
		label: Some(label),
		color_attachments: &[Some(wgpu::RenderPassColorAttachment {
			view: &target.view,
			depth_slice: None,
			resolve_target: None,
			ops: wgpu::Operations {
				load,
				store: wgpu::StoreOp::Store,
			},
		})],
		depth_stencil_attachment: None,
		timestamp_writes: None,
		occlusion_query_set: None,
		multiview_mask: None,
	})
}

/// A run of consecutive quads drawn by one call, and the atlases bound for it.
#[derive(Debug, PartialEq)]
struct Call {
	quads: std::ops::Range<usize>,
	slots: Vec<usize>,
}

/// Split `quads` into draw calls of at most [`SLOTS`] distinct atlases, keeping order.
fn batch(quads: &[Quad]) -> Vec<Call> {
	let mut calls: Vec<Call> = Vec::new();
	for (index, quad) in quads.iter().enumerate() {
		let fits = calls
			.last()
			.is_some_and(|call| call.slots.contains(&quad.texture) || call.slots.len() < SLOTS);
		if !fits {
			calls.push(Call {
				quads: index..index,
				slots: Vec::new(),
			});
		}
		let call = calls.last_mut().expect("a call exists");
		if !call.slots.contains(&quad.texture) {
			call.slots.push(quad.texture);
		}
		call.quads.end = index + 1;
	}
	calls
}

/// Quad `quad`'s four vertices, sampling atlas `slot`, onto `out`.
fn push_vertices(out: &mut Vec<u8>, quad: &Quad, slot: u32) {
	for corner in 0..4 {
		out.extend_from_slice(&vertex(quad, corner, slot));
	}
}

/// Quad `quad`'s vertex at `corner`, sampling atlas `slot`, as the shader reads it.
fn vertex(quad: &Quad, corner: usize, slot: u32) -> [u8; VERTEX_SIZE as usize] {
	let mut out = [0u8; VERTEX_SIZE as usize];
	let mut at = 0;
	let mut put = |bytes: &[u8]| {
		out[at..at + bytes.len()].copy_from_slice(bytes);
		at += bytes.len();
	};
	let [x, y] = quad.corners[corner];
	for value in [x, y, 0.0, 1.0] {
		put(&value.to_le_bytes());
	}
	let color = quad.corner_colors.map_or(quad.color, |colors| colors[corner]);
	put(&color);
	put(&quad.overlay);
	for value in quad.uv[corner] {
		put(&value.to_le_bytes());
	}
	put(&slot.to_le_bytes());
	put(&quad.saturation.to_le_bytes());
	put(&quad.factor.to_le_bytes());
	put(&quad.flags.to_le_bytes());
	put(&quad.level.to_le_bytes());
	out
}

/// The buffers the draws stream their quads through, kept from draw to draw (a buffer
/// made and filled per draw cost more than drawing a zoomed-out world map).
struct Streams {
	vertices: Stream,
	indices: QuadIndices,
	/// The vertices' bytes as they're built.
	scratch: Vec<u8>,
	kept: Kept,
	instances: instances::Instances,
}

/// Quads kept on the GPU from frame to frame (the world map's terrain and details, see
/// `crate::kept_map`), and the writes to them waiting for the next draw.
#[derive(Default)]
struct Kept {
	buffer: Option<wgpu::Buffer>,
	/// Each write's offset in the buffer and its bytes in `bytes`.
	writes: Vec<(u64, std::ops::Range<usize>)>,
	bytes: Vec<u8>,
}

impl Streams {
	fn new(gpu: &Gpu) -> Streams {
		Streams {
			vertices: Stream {
				buffer: None,
				belt: wgpu::util::StagingBelt::new(gpu.device.clone(), STAGING_CHUNK),
			},
			indices: QuadIndices::default(),
			scratch: Vec::new(),
			kept: Kept::default(),
			instances: instances::Instances::default(),
		}
	}
}

/// The staging memory's unit: a zoomed-out world map's quads are about 7 MB.
const STAGING_CHUNK: u64 = 8 << 20;

/// A vertex buffer rewritten by each draw, grown as needed, and filled through a staging
/// belt whose mapped chunks are reused from draw to draw (`Queue::write_buffer` makes new
/// staging memory for every write, and touching megabytes of fresh pages each frame cost
/// several milliseconds). Reusing the buffer between the draws of a frame is safe: each
/// draw's copy is recorded in its own submission, after the previous draw's.
struct Stream {
	buffer: Option<wgpu::Buffer>,
	belt: wgpu::util::StagingBelt,
}

impl Stream {
	/// The buffer holding `bytes` (a multiple of four) from its start, once `encoder`'s
	/// copy runs; the belt must then be finished with `encoder` before it's submitted.
	fn write(&mut self, gpu: &Gpu, encoder: &mut wgpu::CommandEncoder, bytes: &[u8]) -> &wgpu::Buffer {
		let size = (bytes.len() as u64).max(4).next_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT);
		if self.buffer.as_ref().is_none_or(|b| b.size() < size) {
			self.buffer = Some(gpu.device.create_buffer(&wgpu::BufferDescriptor {
				label: Some("sprite vertices"),
				size: size.next_power_of_two(),
				usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
				mapped_at_creation: false,
			}));
		}
		let buffer = self.buffer.as_ref().expect("made above");
		if let Some(size) = wgpu::BufferSize::new(bytes.len() as u64) {
			self.belt.write_buffer(encoder, buffer, 0, size).copy_from_slice(bytes);
		}
		buffer
	}
}

/// The index buffer for drawing quads: quad `i` is the triangles `4i, 4i+1, 4i+2` and
/// `4i, 4i+2, 4i+3`. The same for every draw, so it's only written when it grows.
#[derive(Default)]
struct QuadIndices {
	buffer: Option<wgpu::Buffer>,
	quads: usize,
}

impl QuadIndices {
	/// A buffer indexing at least `quads` quads.
	fn quads(&mut self, gpu: &Gpu, quads: usize) -> &wgpu::Buffer {
		if self.buffer.is_none() || self.quads < quads {
			let quads = quads.max(1024).next_power_of_two();
			let bytes: Vec<u8> = (0..quads as u32)
				.flat_map(|q| {
					let base = q * 4;
					[base, base + 1, base + 2, base, base + 2, base + 3]
				})
				.flat_map(u32::to_le_bytes)
				.collect();
			let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
				label: Some("sprite indices"),
				size: bytes.len() as u64,
				usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
				mapped_at_creation: false,
			});
			gpu.queue.write_buffer(&buffer, 0, &bytes);
			self.buffer = Some(buffer);
			self.quads = quads;
		}
		self.buffer.as_ref().expect("made above")
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn quad(texture: usize) -> Quad {
		Quad {
			texture,
			corners: [[0.0; 2]; 4],
			uv: [[0.0; 2]; 4],
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
	fn quads_share_a_call_until_a_ninth_atlas() {
		let mut quads: Vec<Quad> = (0..8).map(quad).collect();
		quads.push(quad(3)); // already bound
		quads.push(quad(8)); // ninth distinct atlas
		quads.push(quad(0));
		let calls = batch(&quads);
		assert_eq!(calls.len(), 2);
		assert_eq!(
			calls[0],
			Call {
				quads: 0..9,
				slots: (0..8).collect()
			}
		);
		assert_eq!(
			calls[1],
			Call {
				quads: 9..11,
				slots: vec![8, 0]
			}
		);
	}

	#[test]
	fn nothing_to_draw_makes_no_calls() {
		assert!(batch(&[]).is_empty());
	}

	#[test]
	fn uniforms_follow_the_shader_layout() {
		let scene = SceneUniforms {
			translation: [1.0, 2.0],
			scale: [3.0, 4.0, 5.0],
			ambient_color: [6.0, 7.0, 8.0, 9.0],
			ambient_saturation: 10.0,
			camera_level: 11.0,
			dof_strength: 12.0,
		};
		let floats: Vec<f32> = scene
			.bytes()
			.chunks(4)
			.map(|c| f32::from_le_bytes(c.try_into().unwrap()))
			.collect();
		assert_eq!(
			floats,
			[1.0, 2.0, 0.0, 0.0, 3.0, 4.0, 5.0, 10.0, 6.0, 7.0, 8.0, 9.0, 11.0, 12.0, 0.0, 0.0]
		);
	}
}
