//! Quads kept on the GPU as instances of the sprite program (`instanced.wgsl`): the
//! world map's moving sprites (see `crate::kept_sprites`).
//!
//! Each quad is a record in a storage buffer, at a place its owner keeps for it, and is
//! written again only when it changes. A frame draws them through an order list (the
//! instances' places in drawing order, each with its draw call's atlas slot), which the
//! owner uploads only when it differs from the last frame's. The writes wait here for
//! the next draw, which records them on its encoder.

use std::num::NonZeroU64;
use std::ops::Range;

use crate::gpu::Gpu;
use crate::sprite::Quad;

/// Bytes of a record, as `instanced.wgsl` reads it: four corners and four texture
/// coordinates (2 × f32 each), the colour and the overlay colour (4 × f32 each),
/// saturation and factor (f32), flags and level (u32).
pub(crate) const RECORD: usize = 112;

/// An order list entry's atlas slot bits (see [`Instances::set_order`]).
pub(crate) const SLOT_BITS: u32 = 3;

/// The instances' buffers and the writes waiting for the next draw.
#[derive(Default)]
pub(crate) struct Instances {
	records: Option<wgpu::Buffer>,
	order: Option<wgpu::Buffer>,
	/// The group binding both (made again when either is).
	group: Option<wgpu::BindGroup>,
	/// Each write's offset in the records and its bytes in `bytes`.
	writes: Vec<(u64, Range<usize>)>,
	bytes: Vec<u8>,
	/// The order list to upload with the next draw.
	order_bytes: Option<Vec<u8>>,
}

/// The bind group layout of the instances' buffers (group 2 of the instanced pipeline).
pub(crate) fn layout(gpu: &Gpu) -> wgpu::BindGroupLayout {
	let entry = |binding, size| wgpu::BindGroupLayoutEntry {
		binding,
		visibility: wgpu::ShaderStages::VERTEX,
		ty: wgpu::BindingType::Buffer {
			ty: wgpu::BufferBindingType::Storage { read_only: true },
			has_dynamic_offset: false,
			min_binding_size: NonZeroU64::new(size),
		},
		count: None,
	};
	gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
		label: Some("sprite instances"),
		entries: &[entry(0, RECORD as u64), entry(1, 4)],
	})
}

impl Instances {
	/// The bytes of its buffers on the GPU.
	pub(crate) fn bytes(&self) -> u64 {
		[&self.records, &self.order].iter().filter_map(|b| b.as_ref()).map(wgpu::Buffer::size).sum()
	}

	/// Keep room for `count` instances: whether the room was made anew, every instance
	/// to be written again.
	pub(crate) fn keep(&mut self, gpu: &Gpu, count: usize) -> bool {
		let size = (count.max(1) * RECORD) as u64;
		if self.records.as_ref().is_some_and(|b| b.size() >= size) {
			return false;
		}
		self.records = Some(gpu.device.create_buffer(&wgpu::BufferDescriptor {
			label: Some("kept sprite instances"),
			size: size.next_power_of_two(),
			usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
			mapped_at_creation: false,
		}));
		self.group = None;
		self.writes.clear();
		self.bytes.clear();
		true
	}

	/// Write instance `at` as `quad` with the next draw.
	pub(crate) fn write(&mut self, at: usize, quad: &Quad) {
		let start = self.bytes.len();
		push_record(&mut self.bytes, quad);
		let end = self.bytes.len();
		let offset = (at * RECORD) as u64;
		// A write right after the last one goes with it.
		match self.writes.last_mut() {
			Some((last_at, range)) if *last_at + range.len() as u64 == offset && range.end == start => range.end = end,
			_ => self.writes.push((offset, start..end)),
		}
	}

	/// Draw from the order list `entries` from the next draw on: each an instance's index
	/// shifted left by [`SLOT_BITS`], or'd with its call's atlas slot.
	pub(crate) fn set_order(&mut self, entries: &[u32]) {
		self.order_bytes = Some(entries.iter().flat_map(|e| e.to_le_bytes()).collect());
	}

	/// Record the writes waiting on `encoder` (through `belt`), and give the group to
	/// draw with (`None` before anything was kept).
	pub(crate) fn upload(
		&mut self,
		gpu: &Gpu,
		layout: &wgpu::BindGroupLayout,
		encoder: &mut wgpu::CommandEncoder,
		belt: &mut wgpu::util::StagingBelt,
	) -> Option<wgpu::BindGroup> {
		let records = self.records.as_ref()?;
		for (at, range) in self.writes.drain(..) {
			if let Some(size) = wgpu::BufferSize::new(range.len() as u64) {
				belt.write_buffer(encoder, records, at, size).copy_from_slice(&self.bytes[range]);
			}
		}
		self.bytes.clear();
		if let Some(order) = self.order_bytes.take() {
			let size = (order.len() as u64).max(4).next_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT);
			if self.order.as_ref().is_none_or(|b| b.size() < size) {
				self.order = Some(gpu.device.create_buffer(&wgpu::BufferDescriptor {
					label: Some("kept sprite order"),
					size: size.next_power_of_two(),
					usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
					mapped_at_creation: false,
				}));
				self.group = None;
			}
			if let Some(size) = wgpu::BufferSize::new(order.len() as u64) {
				belt.write_buffer(encoder, self.order.as_ref().expect("made above"), 0, size).copy_from_slice(&order);
			}
		}
		let order = self.order.as_ref()?;
		let group = self.group.get_or_insert_with(|| {
			gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
				label: Some("sprite instances"),
				layout,
				entries: &[
					wgpu::BindGroupEntry { binding: 0, resource: records.as_entire_binding() },
					wgpu::BindGroupEntry { binding: 1, resource: order.as_entire_binding() },
				],
			})
		});
		Some(group.clone())
	}
}

/// `quad`'s record onto `out`. Its colours go as the vertex fetch reads the bytes of a
/// vertex-buffer quad (`Unorm8x4`: each byte over 255).
fn push_record(out: &mut Vec<u8>, quad: &Quad) {
	let start = out.len();
	for [x, y] in quad.corners {
		out.extend_from_slice(&x.to_le_bytes());
		out.extend_from_slice(&y.to_le_bytes());
	}
	for [u, v] in quad.uv {
		out.extend_from_slice(&u.to_le_bytes());
		out.extend_from_slice(&v.to_le_bytes());
	}
	for c in quad.color.into_iter().chain(quad.overlay) {
		out.extend_from_slice(&(f32::from(c) / 255.0).to_le_bytes());
	}
	out.extend_from_slice(&quad.saturation.to_le_bytes());
	out.extend_from_slice(&quad.factor.to_le_bytes());
	out.extend_from_slice(&quad.flags.to_le_bytes());
	out.extend_from_slice(&quad.level.to_le_bytes());
	debug_assert_eq!(out.len() - start, RECORD);
	debug_assert!(quad.corner_colors.is_none(), "a kept instance has one colour");
}
