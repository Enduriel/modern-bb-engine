//! The world map's fog of war: the original's `fow` program, ported to WGSL.
//!
//! Undiscovered land is covered by a cloudy grey texture (`gfx/fow.png`). Where the
//! fog lies comes from the engine's fog mask (`squirrel_bb::FogPatch`), which
//! `uncoverFogOfWar` stamps clear. It spans the map's extent, and this pass takes its
//! opacity from it. As the original's pass (`0x4a24c0`) does,
//! it is a [`WorldPass`]:
//! - the fog texture's coordinates are taken over 2048 units, the original's mask size,
//!   not the texture's own (kept at 2048 for a larger map's larger mask, so the clouds
//!   look the same at any map size);
//! - as in `shaders/fow.fs`, three samples at different scales and drift directions
//!   are averaged, times the ambient colour, drifting with `Time.getRealTimeF() / 100`.
//!
//! The original moves the fog on every frame it draws (60 a second). Ours moves it in
//! steps ([`drift_time`]): 60 a second where a step moves it more than a quarter of a
//! screen pixel (as the original's frames do), fewer where it moves slower on the screen
//! (zoomed out), each step then under a quarter pixel. A still map with fog on it needs a
//! new frame only when the fog has moved (the window leaves the last one up in between):
//! zoomed out ~15 frames a second instead of the display's 180.

use squirrel_bb::{FOG_MASK_SIZE, FogPatch};

use crate::gpu::Gpu;
use crate::texture::GpuTexture;
use crate::world::WorldView;
use crate::world_pass::{PassDraw, PassParams, WorldPass};

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

/// The fastest of the three samples' drifts (the second's, `(0.5, 1.5)` texture widths a
/// hundred seconds). Its scale cancels out: a texture taken smaller drifts as much
/// slower over the map. A texture width is [`FOG_MASK_SIZE`] world units at most.
const FASTEST_DRIFT: [f32; 2] = [0.5, 1.5];

/// The fog's steps: at most 60 a second, as the original's frames.
const STEP: f32 = 1.0 / 60.0;

/// The most a step may move the fog, in screen pixels: below one 8-bit level of change
/// for the fog texture's gradients (at most 12 levels a texel, 4 for 99% of it).
const STEP_PIXELS: f32 = 0.25;

/// The real time the fog is drawn at, for a view of `zoom` screen pixels per world unit:
/// `real` (seconds) rounded down to a whole step. A step is [`STEP`], doubled as long as
/// the fog moves no more than [`STEP_PIXELS`] in it (powers of two, so that the times
/// of a coarser step are among a finer one's when the zoom changes). At zoom 1 that is
/// 60 steps a second (32 px a second, ~0.54 px a step, as the original's frames); the
/// camera zoomed out (~0.1) 15.
pub fn drift_time(real: f32, zoom: f32) -> f32 {
	let speed = drift_speed(zoom);
	let mut step = STEP;
	// Up to ~1 s: past that, steps would be too far apart to call the fog moving.
	for _ in 0..6 {
		if speed * step * 2.0 > STEP_PIXELS {
			break;
		}
		step *= 2.0;
	}
	(real / step).floor() * step
}

/// How fast the fog's fastest sample moves on the screen, in pixels a second, at `zoom`.
fn drift_speed(zoom: f32) -> f32 {
	FASTEST_DRIFT[0].hypot(FASTEST_DRIFT[1]) * FOG_MASK_SIZE as f32 / 100.0 * zoom
}

/// The fog-of-war compositing pass, and the mask it reads.
pub struct FogPass {
	pass: WorldPass,
	/// The engine's fog mask, once a patch arrived, and its side in texels.
	mask: Option<(wgpu::Texture, wgpu::TextureView, u32)>,
}

impl FogPass {
	pub fn new(gpu: &Gpu) -> Self {
		FogPass {
			pass: WorldPass::new(gpu, "fog", FRAGMENT, true),
			mask: None,
		}
	}

	/// The bytes the fog mask takes on the GPU (none before its first patch).
	pub fn bytes(&self) -> u64 {
		self.mask.as_ref().map_or(0, |(texture, _, _)| crate::texture::texture_bytes(texture))
	}

	/// Copy a changed part of the engine's fog mask to the GPU.
	/// A mask of another size (another map's) replaces the texture.
	pub fn update_mask(&mut self, gpu: &Gpu, patch: &FogPatch) {
		let size = patch.mask;
		if self.mask.as_ref().is_some_and(|m| m.2 != size) {
			self.mask = None;
		}
		let (texture, _, _) = self.mask.get_or_insert_with(|| {
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
			(texture, view, size)
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

	/// The fog's draw over a frame, for a pass of its encoder: `fog` is `gfx/fow.png`,
	/// `map_extent` the world size the mask spans, `real_time` (seconds) drives the
	/// drift (in steps, see [`drift_time`]), `ambient` is the scene's light. Nothing is
	/// drawn before the mask's first patch.
	pub(crate) fn prepare(
		&self,
		gpu: &Gpu,
		view: &WorldView,
		fog: &GpuTexture,
		map_extent: (f32, f32),
		real_time: f32,
		ambient: [f32; 4],
	) -> Option<PassDraw> {
		let (_, mask, _) = self.mask.as_ref()?;
		if !(map_extent.0 > 0.0 && map_extent.1 > 0.0) {
			return None;
		}
		let size = FOG_MASK_SIZE as f32;
		let params = PassParams {
			view,
			map_extent,
			texture_size: (size, size),
			factor: drift_time(real_time, view.zoom) / 100.0,
			ambient,
		};
		Some(self.pass.prepare(gpu, &params, fog, Some(mask)))
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_fog_moves_in_steps_of_at_most_a_quarter_pixel_and_a_sixtieth() {
		let near = |a: f32, b: f32| (a - b).abs() < 1e-4;
		// Zoom 1: 60 steps a second (the drift is 32 px/s, so half a pixel each).
		assert!(near(drift_time(1.0 + 1.0 / 120.0, 1.0), 1.0));
		assert!(near(drift_time(1.0 + 1.0 / 60.0 + 0.001, 1.0), 1.0 + 1.0 / 60.0));
		// Zoomed out (the camera at ~9.8): 3.3 px/s, steps of 4/60 s, 0.22 px each.
		let zoom = 1.0 / 9.77;
		assert!(near(drift_time(10.0 + 0.06, zoom), 10.0));
		assert!(near(drift_time(10.0 + 0.07, zoom), 10.0 + 4.0 / 60.0));
		for zoom in [4.0, 1.0, 0.5, 0.25, 0.1, 0.05, 0.001] {
			let speed = drift_speed(zoom);
			let (mut last, mut steps) = (drift_time(100.0, zoom), 0);
			for i in 1..=6000 {
				let now = drift_time(100.0 + i as f32 / 1000.0, zoom);
				if now != last {
					assert!((now - last) * speed <= STEP_PIXELS.max(speed * STEP) + 1e-3, "zoom {zoom}");
					steps += 1;
					last = now;
				}
			}
			assert!(steps <= 6 * 60 && steps >= 5, "zoom {zoom}: {steps} steps in 6 s");
		}
	}
}
