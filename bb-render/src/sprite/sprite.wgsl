// The original's sprite program (shaders/sprite.vs + sprite.fs), ported to WGSL.
//
// Positions are world units; the vertex stage subtracts the camera translation and
// multiplies by the scale straight into clip space. Each vertex picks one of eight
// atlases by index. GLSL may sample inside that branch; WGSL needs uniform control
// flow for implicit derivatives, so the derivatives are taken first and the branch
// samples with them explicitly (the same texels and mip level).
//
// With a depth-of-field strength (the battle's `sprite_dof` program) a sprite levels
// below the camera blends in its mip level 2: `mix(lod2, texel, 1 - strength × n)`.

struct Uniforms {
	translation: vec2<f32>,
	scale: vec3<f32>,
	ambient_saturation: f32,
	ambient_color: vec4<f32>,
	// x: the camera's level, y: the depth of field's strength.
	dof: vec4<f32>,
}

@group(0) @binding(0) var<uniform> u: Uniforms;

@group(1) @binding(0) var tex0: texture_2d<f32>;
@group(1) @binding(1) var tex1: texture_2d<f32>;
@group(1) @binding(2) var tex2: texture_2d<f32>;
@group(1) @binding(3) var tex3: texture_2d<f32>;
@group(1) @binding(4) var tex4: texture_2d<f32>;
@group(1) @binding(5) var tex5: texture_2d<f32>;
@group(1) @binding(6) var tex6: texture_2d<f32>;
@group(1) @binding(7) var tex7: texture_2d<f32>;
@group(1) @binding(8) var smp0: sampler;
@group(1) @binding(9) var smp1: sampler;
@group(1) @binding(10) var smp2: sampler;
@group(1) @binding(11) var smp3: sampler;
@group(1) @binding(12) var smp4: sampler;
@group(1) @binding(13) var smp5: sampler;
@group(1) @binding(14) var smp6: sampler;
@group(1) @binding(15) var smp7: sampler;

struct VertexIn {
	@location(0) position: vec4<f32>,
	@location(1) color0: vec4<f32>,
	@location(2) color1: vec4<f32>,
	@location(3) tex_coords: vec2<f32>,
	@location(4) texture: u32,
	@location(5) saturation: f32,
	@location(6) factor: f32,
	@location(7) flags: u32,
	@location(8) level: u32,
}

struct VertexOut {
	@builtin(position) position: vec4<f32>,
	@location(0) color0: vec4<f32>,
	@location(1) color1: vec4<f32>,
	@location(2) tex_coords: vec2<f32>,
	@location(3) @interpolate(flat) texture: u32,
	@location(4) saturation: f32,
	@location(5) factor: f32,
	@location(6) @interpolate(flat) flags: u32,
	@location(7) dof: f32,
}

@vertex
fn vs_main(v: VertexIn) -> VertexOut {
	var out: VertexOut;
	var position = v.position;
	position.x = (position.x - u.translation.x) * u.scale.x;
	position.y = (position.y - u.translation.y) * u.scale.y;
	position.z = position.z * u.scale.z;
	out.position = position;
	// The colours arrive as unsigned-normalized bytes: the original's `/ 255.0`.
	out.color0 = v.color0;
	out.color1 = v.color1;
	out.tex_coords = v.tex_coords;
	out.texture = v.texture;
	out.saturation = v.saturation;
	out.factor = v.factor;
	out.flags = v.flags;
	out.dof = min(1.0, 1.0 - u.dof.y * max(0.0, u.dof.x - f32(v.level)));
	return out;
}

fn sample_atlas_lod2(index: u32, uv: vec2<f32>) -> vec4<f32> {
	switch index {
		case 0u: { return textureSampleLevel(tex0, smp0, uv, 2.0); }
		case 1u: { return textureSampleLevel(tex1, smp1, uv, 2.0); }
		case 2u: { return textureSampleLevel(tex2, smp2, uv, 2.0); }
		case 3u: { return textureSampleLevel(tex3, smp3, uv, 2.0); }
		case 4u: { return textureSampleLevel(tex4, smp4, uv, 2.0); }
		case 5u: { return textureSampleLevel(tex5, smp5, uv, 2.0); }
		case 6u: { return textureSampleLevel(tex6, smp6, uv, 2.0); }
		case 7u: { return textureSampleLevel(tex7, smp7, uv, 2.0); }
		default: { return vec4<f32>(0.0); }
	}
}

fn sample_atlas(index: u32, uv: vec2<f32>, ddx: vec2<f32>, ddy: vec2<f32>) -> vec4<f32> {
	switch index {
		case 0u: { return textureSampleGrad(tex0, smp0, uv, ddx, ddy); }
		case 1u: { return textureSampleGrad(tex1, smp1, uv, ddx, ddy); }
		case 2u: { return textureSampleGrad(tex2, smp2, uv, ddx, ddy); }
		case 3u: { return textureSampleGrad(tex3, smp3, uv, ddx, ddy); }
		case 4u: { return textureSampleGrad(tex4, smp4, uv, ddx, ddy); }
		case 5u: { return textureSampleGrad(tex5, smp5, uv, ddx, ddy); }
		case 6u: { return textureSampleGrad(tex6, smp6, uv, ddx, ddy); }
		case 7u: { return textureSampleGrad(tex7, smp7, uv, ddx, ddy); }
		// The original leaves the texel undefined for other indices.
		default: { return vec4<f32>(0.0); }
	}
}

@fragment
fn fs_main(f: VertexOut) -> @location(0) vec4<f32> {
	let ddx = dpdx(f.tex_coords);
	let ddy = dpdy(f.tex_coords);
	let tex0 = sample_atlas(f.texture, f.tex_coords, ddx, ddy);

	var col = f.color0 * tex0;
	if f.dof < 1.0 {
		col = f.color0 * mix(sample_atlas_lod2(f.texture, f.tex_coords), tex0, f.dof);
	}
	let saturation = f.saturation * u.ambient_saturation;
	if saturation != 1.0 {
		let desaturated = (1.0 - saturation) * (col.r + col.g + col.b) * 0.333333333;
		col = vec4<f32>(col.rgb * saturation + desaturated, col.a);
	}
	if f.factor != 0.0 {
		col = vec4<f32>(col.rgb * (1.0 - f.factor) + f.color1.rgb * f.factor, col.a);
	}
	if f.flags == 0u {
		return u.ambient_color * col;
	}
	return col;
}
