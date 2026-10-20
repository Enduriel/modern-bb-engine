//! The world map's water: the original's `water` program, ported to WGSL.
//!
//! Ocean and shore tiles have no terrain brush. Before anything else, the original
//! covers the screen with `gfx/water.png` (`World.prepareRender()`, `0x4e37a0`)
//! as a [`WorldPass`] over the texture's own size, and the
//! map draws over it. As in `shaders/water.fs`, three samples at different scales and
//! drift directions are averaged, alpha included, times the ambient colour. The drift
//! factor is the clock object's virtual time in seconds (`+0x20`, which advances by
//! the frame time times the virtual speed) over 130.

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
	let c2 = textureSample(tex, tex_sampler, (uv + k * vec2<f32>(0.5, 0.75)) * 0.33);
	let c3 = textureSample(tex, tex_sampler, (uv + k * vec2<f32>(1.25, 0.5)) * 1.75);
	return (c1 * 0.33 + c2 * 0.33 + c3 * 0.33) * u.ambient;
}
"#;

/// The water pass.
pub struct WaterPass {
	pass: WorldPass,
}

impl WaterPass {
	pub fn new(gpu: &Gpu) -> Self {
		WaterPass {
			pass: WorldPass::new(gpu, "water", FRAGMENT, false),
		}
	}

	/// The water's draw over a frame, for a pass of its encoder: `water` is
	/// `gfx/water.png`, `map_extent` the map's extent, `virtual_time` (seconds) drives
	/// the drift, `ambient` is the scene's light. `None` without a map: nothing is
	/// drawn.
	pub(crate) fn prepare(
		&self,
		gpu: &Gpu,
		view: &WorldView,
		water: &GpuTexture,
		map_extent: (f32, f32),
		virtual_time: f32,
		ambient: [f32; 4],
	) -> Option<PassDraw> {
		if !(map_extent.0 > 0.0 && map_extent.1 > 0.0) {
			return None;
		}
		let params = PassParams {
			view,
			map_extent,
			texture_size: (water.width as f32, water.height as f32),
			factor: virtual_time / 130.0,
			ambient,
		};
		Some(self.pass.prepare(gpu, &params, water, None))
	}
}
