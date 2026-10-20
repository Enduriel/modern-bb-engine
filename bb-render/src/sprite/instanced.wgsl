// The sprite program's vertex stage for quads kept on the GPU as instances (the world
// map's moving sprites, `crate::kept_sprites`), appended to `sprite.wgsl`.
//
// Each quad is one record in `instances`, written once and again only when it changes;
// a frame draws them in the order of `order`, a list the CPU sorts and uploads, whose
// entries are an instance's index shifted left by 3 and the atlas slot its draw call
// binds it in. An instance draws as two triangles, the same as a quad of the vertex
// buffer (0 1 2, 0 2 3), from the same values, so the pixels are the same.

struct Instance {
	corners: array<vec2<f32>, 4>,
	uv: array<vec2<f32>, 4>,
	color0: vec4<f32>,
	color1: vec4<f32>,
	saturation: f32,
	factor: f32,
	flags: u32,
	level: u32,
}

@group(2) @binding(0) var<storage, read> instances: array<Instance>;
@group(2) @binding(1) var<storage, read> order: array<u32>;

@vertex
fn vs_instanced(@builtin(vertex_index) vertex: u32, @builtin(instance_index) at: u32) -> VertexOut {
	var corners = array<u32, 6>(0u, 1u, 2u, 0u, 2u, 3u);
	let corner = corners[vertex];
	let entry = order[at];
	let q = instances[entry >> 3u];
	var v: VertexIn;
	v.position = vec4<f32>(q.corners[corner], 0.0, 1.0);
	v.color0 = q.color0;
	v.color1 = q.color1;
	v.tex_coords = q.uv[corner];
	v.texture = entry & 7u;
	v.saturation = q.saturation;
	v.factor = q.factor;
	v.flags = q.flags;
	v.level = q.level;
	return vertex_out(v);
}
