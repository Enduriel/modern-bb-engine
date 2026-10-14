//! The world map's fog of war: the original's `fow` program, ported to WGSL.
//!
//! Undiscovered land is covered by a cloudy grey texture (`gfx/fow.png`). Where the
//! fog lies comes from the engine's fog mask (`squirrel_bb::FogPatch`), which
//! `uncoverFogOfWar` stamps clear. It spans the map's extent, and this pass takes its
//! opacity from it. As the original's pass (`0x4a24c0`) does,
//! it is a [`WorldPass`]:
//! - the fog texture's coordinates are taken over 2048 units, the mask's size, not
//!   the texture's own;
//! - as in `shaders/fow.fs`, three samples at different scales and drift directions
//!   are averaged, times the ambient colour, drifting with `Time.getRealTimeF() / 100`.

use squirrel_bb::{FOG_MASK_SIZE, FogPatch};

use crate::gpu::Gpu;
use crate::target::RenderTarget;
use crate::texture::GpuTexture;
use crate::world::WorldView;
use crate::world_pass::{PassParams, WorldPass};

const FRAGMENT: &str = r#"
@fragment
fn fs_main(f: Out) -> @location(0) vec4<f32> {
	let k = u.factor;
	let uv = f.uv;
	let c1 = textureSample(tex, tex_sampler, uv + k * vec2<f32>(1.0, -0.25));
	let c2 = textureSample(tex, tex_sampler, (uv + k * vec2<f32>(0.5, 1.5)) * 0.33);
	let c3 = textureSample(tex, tex_sampler, (uv + k * vec2<f32>(1.25, 0.9)) * 1.75);
	let rgb = (c1.rgb * 0.33 + c2.rgb * 0.33 + c3.rgb * 0.33) * u.ambient.rgb;
	return vec4<f32>(rgb, textureSample(mask, mask_sampler, f.map_uv).r);
}
"#;

/// The fog-of-war compositing pass, and the mask it reads.
pub struct FogPass {
	pass: WorldPass,
	/// The engine's fog mask, once a patch arrived.
	mask: Option<(wgpu::Texture, wgpu::TextureView)>,
}

impl FogPass {
	pub fn new(gpu: &Gpu) -> Self {
		FogPass {
			pass: WorldPass::new(gpu, "fog", FRAGMENT, true),
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
	/// the mask spans, `real_time` (seconds) drives the drift, `ambient` is the scene's
	/// light. Nothing is drawn before the mask's first patch.
	#[allow(clippy::too_many_arguments)]
	pub fn draw(
		&self,
		gpu: &Gpu,
		target: &RenderTarget,
		view: &WorldView,
		fog: &GpuTexture,
		map_extent: (f32, f32),
		real_time: f32,
		ambient: [f32; 4],
	) {
		let Some((_, mask)) = &self.mask else { return };
		if !(map_extent.0 > 0.0 && map_extent.1 > 0.0) {
			return;
		}
		let size = FOG_MASK_SIZE as f32;
		let params = PassParams {
			view,
			map_extent,
			texture_size: (size, size),
			factor: real_time / 100.0,
			ambient,
		};
		self.pass.draw(gpu, target, None, &params, fog, Some(mask));
	}
}
