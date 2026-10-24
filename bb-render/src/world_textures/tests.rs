use super::*;
use crate::block::{self, Plane};
use crate::target::RenderTarget;

const SQUARE: (u32, u32) = (2048, 2048);
const GREY: Content = Content { grey: true, opaque: true };
const COLOUR: Content = Content { grey: false, opaque: true };

#[test]
fn choices_parse() {
	assert_eq!(Choice::parse("rgba8"), Ok(Choice::RGBA8));
	assert_eq!(Choice::parse("R8"), Ok(Choice { storage: Storage::R8, ..Choice::RGBA8 }));
	assert_eq!(
		Choice::parse("bc1-half-flat"),
		Ok(Choice { storage: Storage::Bc1, half: true, mips: false })
	);
	assert!(Choice::parse("bc7").is_err());
	assert!(Choice::parse("bc4-small").is_err());
}

#[test]
fn what_the_content_allows() {
	let image = |rgba: Vec<u8>| Image { width: 2, height: 1, rgba };
	assert_eq!(Content::of(&image(vec![9, 9, 9, 255, 200, 200, 200, 255])), GREY);
	assert_eq!(Content::of(&image(vec![9, 9, 9, 255, 200, 201, 200, 255])), COLOUR);
	assert_eq!(Content::of(&image(vec![9, 9, 9, 254, 2, 2, 2, 255])), Content { grey: true, opaque: false });
}

#[test]
fn the_wanted_format_where_the_content_and_gpu_allow_it() {
	for pass in [Pass::Water, Pass::Fog] {
		for storage in [Storage::Rgba8, Storage::R8, Storage::Bc1, Storage::Bc4] {
			assert_eq!(resolve(pass, storage, GREY, SQUARE, true), storage, "{pass:?} {storage:?}");
		}
	}
	assert_eq!(resolve(Pass::Water, Storage::Bc1, COLOUR, SQUARE, true), Storage::Bc1);
	// The fog's pass reads no alpha: a translucent texture may lose it.
	let translucent = Content { grey: true, opaque: false };
	assert_eq!(resolve(Pass::Fog, Storage::Bc1, translucent, SQUARE, true), Storage::Bc1);
	assert_eq!(resolve(Pass::Fog, Storage::Bc4, translucent, SQUARE, true), Storage::Bc4);
}

#[test]
fn what_the_content_rules_out_falls_back() {
	// Colour (a mod's fog) isn't kept in one channel.
	assert_eq!(resolve(Pass::Fog, Storage::R8, COLOUR, SQUARE, true), Storage::Rgba8);
	assert_eq!(resolve(Pass::Fog, Storage::Bc4, COLOUR, SQUARE, true), Storage::Rgba8);
	// The water's pass reads alpha: BC1 (opaque) and one channel would lose it.
	let translucent = Content { grey: true, opaque: false };
	assert_eq!(resolve(Pass::Water, Storage::Bc1, translucent, SQUARE, true), Storage::Rgba8);
	assert_eq!(resolve(Pass::Water, Storage::R8, translucent, SQUARE, true), Storage::Rgba8);
	assert_eq!(resolve(Pass::Water, Storage::Bc4, translucent, SQUARE, true), Storage::Rgba8);
}

#[test]
fn without_bc_compression_or_whole_blocks_it_falls_back() {
	for (bc, size) in [(false, SQUARE), (true, (2046, 2048)), (true, (2048, 1022))] {
		assert_eq!(resolve(Pass::Fog, Storage::Bc4, GREY, size, bc), Storage::R8, "{bc} {size:?}");
		assert_eq!(resolve(Pass::Fog, Storage::Bc1, GREY, size, bc), Storage::R8, "{bc} {size:?}");
		assert_eq!(resolve(Pass::Water, Storage::Bc1, COLOUR, size, bc), Storage::Rgba8, "{bc} {size:?}");
		assert_eq!(resolve(Pass::Fog, Storage::Bc4, COLOUR, size, bc), Storage::Rgba8, "{bc} {size:?}");
	}
}

fn gpu() -> Option<Gpu> {
	Gpu::headless().map_err(|e| bb_engine::harness::skip(&e)).ok()
}

/// A smooth grey or coloured test image, `size` square.
fn image(size: u32, grey: bool) -> Image {
	let mut rgba = Vec::with_capacity((size * size * 4) as usize);
	for y in 0..size {
		for x in 0..size {
			let v = |k: f32| (120.0 + 20.0 * ((x as f32 / 7.0 + k).sin() + (y as f32 / 11.0).cos())) as u8;
			if grey {
				let g = v(0.0);
				rgba.extend_from_slice(&[g, g, g, 255]);
			} else {
				rgba.extend_from_slice(&[v(0.0) / 4, v(1.0) / 2, v(2.0) / 2, 255]);
			}
		}
	}
	Image { width: size, height: size, rgba }
}

/// Level `level` of `texture` as the GPU decodes it, RGBA (one channel read as grey, as
/// the passes do).
fn decoded(gpu: &Gpu, texture: &wgpu::Texture, level: u32) -> Image {
	let size = texture.size().mip_level_size(level, wgpu::TextureDimension::D2);
	let target = RenderTarget::new(gpu, size.width, size.height);
	let shader = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {
		label: Some("decode"),
		source: wgpu::ShaderSource::Wgsl(
			format!(
				r"
@group(0) @binding(0) var t: texture_2d<f32>;
@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {{
	let xy = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
	return vec4<f32>(xy * 2.0 - 1.0, 0.0, 1.0);
}}
@fragment
fn fs_main(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {{
	let c = textureLoad(t, vec2<i32>(p.xy), {level});
	return select(c, vec4<f32>(c.rrr, 1.0), {grey});
}}",
				grey = is_grey(texture.format())
			)
			.into(),
		),
	});
	let pipeline = gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
		label: Some("decode"),
		layout: None,
		vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_main"), compilation_options: Default::default(), buffers: &[] },
		fragment: Some(wgpu::FragmentState {
			module: &shader,
			entry_point: Some("fs_main"),
			compilation_options: Default::default(),
			targets: &[Some(crate::TARGET_FORMAT.into())],
		}),
		primitive: wgpu::PrimitiveState::default(),
		depth_stencil: None,
		multisample: wgpu::MultisampleState::default(),
		multiview_mask: None,
		cache: None,
	});
	let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
	let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
		label: Some("decode"),
		layout: &pipeline.get_bind_group_layout(0),
		entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) }],
	});
	let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
	{
		let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
			label: Some("decode"),
			color_attachments: &[Some(wgpu::RenderPassColorAttachment {
				view: &target.view,
				depth_slice: None,
				resolve_target: None,
				ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
			})],
			depth_stencil_attachment: None,
			timestamp_writes: None,
			occlusion_query_set: None,
			multiview_mask: None,
		});
		pass.set_pipeline(&pipeline);
		pass.set_bind_group(0, &group, &[]);
		pass.draw(0..3, 0..1);
	}
	gpu.queue.submit([encoder.finish()]);
	target.read(gpu).expect("read back")
}

/// The largest difference of any channel between `a` and `b`.
fn max_diff(a: &Image, b: &Image) -> u8 {
	assert_eq!((a.width, a.height), (b.width, b.height));
	a.rgba.iter().zip(&b.rgba).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0)
}

fn formats(water: &str, fog: &str, bc: bool) -> WorldFormats {
	WorldFormats { water: Choice::parse(water).unwrap(), fog: Choice::parse(fog).unwrap(), bc }
}

#[test]
fn one_channel_is_exactly_the_default_for_a_grey_texture() {
	let Some(gpu) = gpu() else { return };
	let mips = MipMaker::new(&gpu);
	let image = image(64, true);
	let default = upload(&gpu, &mips, formats("rgba8", "rgba8", false), &LevelsAhead::default(), Pass::Fog, "fog", &image);
	let grey = upload(&gpu, &mips, formats("rgba8", "r8", false), &LevelsAhead::default(), Pass::Fog, "fog", &image);
	assert_eq!(grey.texture.format(), wgpu::TextureFormat::R8Unorm);
	assert_eq!(grey.texture.mip_level_count(), default.texture.mip_level_count());
	assert_eq!(grey.bytes() * 4, default.bytes());
	for level in 0..default.texture.mip_level_count() {
		assert_eq!(max_diff(&decoded(&gpu, &grey.texture, level), &decoded(&gpu, &default.texture, level)), 0, "level {level}");
	}
}

#[test]
fn block_compressed_levels_decode_on_the_gpu_as_the_reference_does() {
	let Some(gpu) = gpu() else { return };
	if !gpu.device.features().contains(wgpu::Features::TEXTURE_COMPRESSION_BC) {
		bb_engine::harness::skip(&format!("{} lacks BC texture compression", gpu.adapter));
		return;
	}
	let mips = MipMaker::new(&gpu);
	for (pass, choice, grey) in [(Pass::Water, "bc1", false), (Pass::Fog, "bc4", true)] {
		let image = image(64, grey);
		let texture = upload(&gpu, &mips, formats(choice, choice, true), &LevelsAhead::default(), pass, "test", &image);
		assert_eq!(texture.texture.format(), Choice::parse(choice).unwrap().storage.format());
		assert_eq!(texture.texture.mip_level_count(), 7);
		// 8 bytes a 4×4 block: 64², 32², 16², 8², 4², and 2², 1² a block each.
		assert_eq!(texture.bytes(), (256 + 64 + 16 + 4 + 1 + 1 + 1) * 8);
		let chain = bb_assets::mips::mip_chain(&image);
		for (level, want) in chain.iter().enumerate() {
			let plane = Plane::rgba(want);
			let reference = match pass {
				Pass::Water => block::decode_bc1(want.width, want.height, &block::encode_bc1(&plane, 2)),
				Pass::Fog => block::decode_bc4(want.width, want.height, &block::encode_bc4(&plane, 2)),
			};
			let got = decoded(&gpu, &texture.texture, level as u32);
			// The GPU interpolates within the formats' tolerance of the exact palette.
			assert!(max_diff(&got, &reference) <= 2, "{choice} level {level}: {}", max_diff(&got, &reference));
			// And the encoding stays close to the source on the levels where the test
			// image is smooth (a wave spans a few blocks).
			if want.width >= 16 {
				let mean = got.rgba.iter().zip(&want.rgba).map(|(a, b)| f64::from(a.abs_diff(*b))).sum::<f64>() / got.rgba.len() as f64;
				let (mean_bound, max_bound) = if grey { (1.0, 4) } else { (2.0, 12) };
				assert!(mean <= mean_bound && max_diff(&got, want) <= max_bound, "{choice} level {level} vs the source: mean {mean}, max {}", max_diff(&got, want));
			}
		}
	}
}

#[test]
fn without_bc_on_the_gpu_the_fog_is_one_channel_and_the_water_rgba8() {
	let Some(gpu) = gpu() else { return };
	let mips = MipMaker::new(&gpu);
	let water = upload(&gpu, &mips, formats("bc1", "bc4", false), &LevelsAhead::default(), Pass::Water, "water", &image(64, false));
	let fog = upload(&gpu, &mips, formats("bc1", "bc4", false), &LevelsAhead::default(), Pass::Fog, "fog", &image(64, true));
	assert_eq!(water.texture.format(), wgpu::TextureFormat::Rgba8Unorm);
	assert_eq!(fog.texture.format(), wgpu::TextureFormat::R8Unorm);
	// A colour fog asked for in one channel stays RGBA8.
	let colour_fog = upload(&gpu, &mips, formats("rgba8", "r8", true), &LevelsAhead::default(), Pass::Fog, "fog", &image(64, false));
	assert_eq!(colour_fog.texture.format(), wgpu::TextureFormat::Rgba8Unorm);
}

#[test]
fn half_and_flat_keep_the_images_size_for_coordinates() {
	let Some(gpu) = gpu() else { return };
	let mips = MipMaker::new(&gpu);
	let half = upload(&gpu, &mips, formats("rgba8-half", "r8-half-flat", false), &LevelsAhead::default(), Pass::Water, "water", &image(64, false));
	assert_eq!((half.width, half.height), (64, 64));
	assert_eq!((half.texture.width(), half.texture.mip_level_count()), (32, 6));
	let flat = upload(&gpu, &mips, formats("rgba8-half", "r8-half-flat", false), &LevelsAhead::default(), Pass::Fog, "fog", &image(64, true));
	assert_eq!((flat.width, flat.texture.width(), flat.texture.mip_level_count()), (64, 32, 1));
}

#[test]
fn by_default_the_water_is_bc1_and_the_fog_bc4() {
	assert_eq!(Pass::Water.default_choice(), Choice::parse("bc1").unwrap());
	assert_eq!(Pass::Fog.default_choice(), Choice::parse("bc4").unwrap());
	assert_eq!(Pass::of(WATER_TEXTURE), Some(Pass::Water));
	assert_eq!(Pass::of(FOG_TEXTURE), Some(Pass::Fog));
	assert_eq!(Pass::of("gfx/water_2.png"), None);
}

#[test]
fn a_worker_makes_the_levels_and_the_upload_takes_them() {
	let Some(gpu) = gpu() else { return };
	let mips = MipMaker::new(&gpu);
	let formats = formats("bc1", "bc4", true);
	let prepared = LevelsAhead::default();
	let hook = prepared.hook(formats);
	let image = image(64, true);
	// Off the GPU thread, as the image loader's workers run it; other paths untouched.
	std::thread::scope(|scope| {
		scope.spawn(|| {
			hook(FOG_TEXTURE, &image);
			hook("gfx/other.png", &image);
		});
	});
	assert!(prepared.0.lock().unwrap().keys().eq([FOG_TEXTURE]));
	let world = WorldTextures { formats, prepared: prepared.clone() };
	let fog = world.upload(&gpu, &mips, FOG_TEXTURE, &image);
	assert!(prepared.0.lock().unwrap().is_empty(), "taken by the upload");
	let format = if gpu.device.features().contains(wgpu::Features::TEXTURE_COMPRESSION_BC) { wgpu::TextureFormat::Bc4RUnorm } else { wgpu::TextureFormat::R8Unorm };
	assert_eq!((fog.texture.format(), fog.texture.mip_level_count()), (format, 7));
	// Without a worker's levels, the upload makes them: the same texture.
	let again = world.upload(&gpu, &mips, FOG_TEXTURE, &image);
	assert_eq!(max_diff(&decoded(&gpu, &again.texture, 0), &decoded(&gpu, &fog.texture, 0)), 0);
	// Anything else goes up as is.
	let other = world.upload(&gpu, &mips, "gfx/other.png", &image);
	assert_eq!((other.texture.format(), other.texture.mip_level_count()), (wgpu::TextureFormat::Rgba8Unorm, 1));
}

#[test]
fn a_fallback_to_rgba8_makes_its_mips_on_the_gpu() {
	let Some(gpu) = gpu() else { return };
	let mips = MipMaker::new(&gpu);
	// BC1 water on a GPU without BC: RGBA8 as before, nothing made on the CPU.
	assert!(make(formats("bc1", "bc4", false), Pass::Water, "water", &image(64, false)).is_none());
	let water = upload(&gpu, &mips, formats("bc1", "bc4", false), &LevelsAhead::default(), Pass::Water, "water", &image(64, false));
	assert_eq!((water.texture.format(), water.texture.mip_level_count()), (wgpu::TextureFormat::Rgba8Unorm, 7));
}
