// The light glows of the cars' environment map (VS_LightGlow / PS_LightGlow of the original):
// every light near the car is a quad facing it at unit distance along the direction to the
// light, half size 0.1 - about 5.7 degrees across whatever the distance. The pixel is the
// light's colour with a round falloff in alpha, (1 - |uv|)^2 x strength, and the blend is
// max(colour x alpha, what is there). (The multiply by alpha is done here: the Max blend of
// this API ignores blend factors, as Direct3D 12's does - the Xbox 360's GPU applies them.)

struct Globals {
    view_proj: mat4x4<f32>,
    cam_pos: vec4<f32>,
};
@group(0) @binding(0) var<uniform> g: Globals;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_env_glow(@builtin(vertex_index) vi: u32, @location(0) pos_str: vec4<f32>, @location(1) color: vec4<f32>) -> VsOut {
    let corner = vec2<f32>(f32(vi & 1u) * 2.0 - 1.0, f32((vi >> 1u) & 1u) * 2.0 - 1.0);
    let dir = normalize(pos_str.xyz - g.cam_pos.xyz);
    let up = select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(dir.y) > 0.99);
    let right = normalize(cross(up, dir));
    let upv = cross(dir, right);
    let world = g.cam_pos.xyz + dir + (right * corner.x + upv * corner.y) * 0.1;
    var out: VsOut;
    out.pos = g.view_proj * vec4<f32>(world, 1.0);
    out.uv = corner;
    out.color = vec4<f32>(color.rgb, pos_str.w);
    return out;
}

@fragment
fn fs_env_glow(in: VsOut) -> @location(0) vec4<f32> {
    let f = 1.0 - clamp(length(in.uv), 0.0, 1.0);
    let a = clamp(f * f * in.color.a, 0.0, 1.0);
    return vec4<f32>(in.color.rgb * a, a);
}
