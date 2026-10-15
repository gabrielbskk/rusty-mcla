// City light sources drawn as additive camera-facing glows (street lamps, signs).

struct Globals {
    view_proj: mat4x4<f32>,
    cam_pos: vec4<f32>,
    sun_dir: vec4<f32>,
    sun_color: vec4<f32>,
    ambient_sky: vec4<f32>,
    ambient_ground: vec4<f32>,
    fog_color: vec4<f32>,
    params: vec4<f32>,
    inv_view_proj: mat4x4<f32>,
    sky_zenith: vec4<f32>,   // a = day factor
    sky_horizon: vec4<f32>,
    sky_sunset: vec4<f32>,
    true_sun: vec4<f32>,
    moon_dir: vec4<f32>,
    light_vp: mat4x4<f32>,
    shadow_params: vec4<f32>,
    cloud_color: vec4<f32>,
    cloud_shade: vec4<f32>,
    tone: vec4<f32>,
    grade: vec4<f32>,
    fog_east: vec4<f32>,
    fog_west: vec4<f32>,
};
@group(0) @binding(0) var<uniform> g: Globals;

// What the tone map (post.wgsl) will turn into this display colour: its colour correction
// undone, then its curve on the luminance, then its exposure.
fn to_scene(d: vec3<f32>) -> vec3<f32> {
    let v = d / max(g.grade.rgb, vec3<f32>(0.001));
    let y = clamp(dot(v, vec3<f32>(0.2125, 0.7154, 0.0721)), 0.0, 0.985);
    let w = max(g.tone.y, 0.001);
    let l = 0.5 * w * (sqrt((1.0 - y) * (1.0 - y) + 4.0 * y / w) - (1.0 - y));
    return v * (l / max(y, 0.0001)) / max(g.tone.x, 0.001);
}

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec3<f32>,
};

@vertex
fn vs_glow(@builtin(vertex_index) vi: u32, @location(6) pos_size: vec4<f32>, @location(7) color: vec4<f32>) -> VsOut {
    var o: VsOut;
    let corner = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u)) - vec2<f32>(1.0); // -1..1 (triangle strip order 0,1,2,3)
    let night = clamp(1.0 - g.sky_zenith.a, 0.0, 1.0);
    let to_cam = g.cam_pos.xyz - pos_size.xyz;
    let dist = length(to_cam);
    // fade out with distance and in daylight
    // a negative size marks a light that also shows in daylight (police light bars)
    let always = step(pos_size.w, 0.0);
    let psize = abs(pos_size.w);
    let fade = clamp((900.0 - dist) / 300.0, 0.0, 1.0) * max(smoothstep(0.25, 0.75, night), always) * clamp(dist / 4.0, 0.0, 1.0);
    let fwd = to_cam / max(dist, 0.001);
    let right = normalize(cross(vec3<f32>(0.0, 1.0, 0.0), fwd) + vec3<f32>(1e-5, 0.0, 0.0));
    let up = cross(fwd, right);
    // keep a minimum apparent size so distant lamps still read as points of light
    // (against the game at night: its lamps are small bright points, not discs of haze)
    let size = min(max(psize * 0.45, dist * 0.003), dist * 0.03) * step(0.001, fade);
    let wp = pos_size.xyz + (right * corner.x + up * corner.y) * size;
    o.clip = g.view_proj * vec4<f32>(wp, 1.0);
    o.uv = corner;
    o.color = color.rgb * color.a * fade;
    return o;
}

@fragment
fn fs_glow(in: VsOut) -> @location(0) vec4<f32> {
    let r = length(in.uv);
    let core = exp(-r * r * 18.0);
    let halo = exp(-r * r * 3.5) * 0.12;
    let a = (core + halo) * (1.0 - smoothstep(0.85, 1.0, r));
    let c = in.color * a * g.params.z;
    // (composed in display terms; added to the HDR scene as what tone-maps to it)
    return vec4<f32>(to_scene(min(c / (c + vec3<f32>(1.0)) * 1.6, vec3<f32>(0.98))), 1.0);
}
