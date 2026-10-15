// The cars' environment cube filled from the two paraboloid halves the city is drawn into
// (city.wgsl vs_env_dp; render/mod.rs `env_dp_views`): each cube texel takes the half its
// direction lies in. Where a half holds nothing (alpha 0: the sky, drawn into the cube face
// before this) the texel is left alone.

@group(0) @binding(0) var t_front: texture_2d<f32>;
@group(0) @binding(1) var t_back: texture_2d<f32>;
@group(0) @binding(2) var s_dp: sampler;
// the garage's light picture (render/mod.rs `set_garage_glow`): the two halves side by side, the
// half that looks along the world's -z on the left; black outside the garage
@group(0) @binding(3) var t_glow: texture_2d<f32>;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) face: u32,
};

@vertex
fn vs_fill(@builtin(vertex_index) vi: u32, @builtin(instance_index) face: u32) -> VsOut {
    let p = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    var out: VsOut;
    out.pos = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2<f32>(p.x, 1.0 - p.y);
    out.face = face;
    return out;
}

fn face_dir(face: u32, uv: vec2<f32>) -> vec3<f32> {
    let s = uv.x * 2.0 - 1.0;
    let t = uv.y * 2.0 - 1.0;
    switch face {
        case 0u { return vec3<f32>(1.0, -t, -s); }
        case 1u { return vec3<f32>(-1.0, -t, s); }
        case 2u { return vec3<f32>(s, 1.0, t); }
        case 3u { return vec3<f32>(s, -1.0, -t); }
        case 4u { return vec3<f32>(s, -t, 1.0); }
        default { return vec3<f32>(-s, -t, -1.0); }
    }
}

@fragment
fn fs_fill(in: VsOut) -> @location(0) vec4<f32> {
    let d = normalize(face_dir(in.face, in.uv));
    let m = vec3<f32>(d.x, d.y, -d.z);
    var c = vec4<f32>(0.0);
    var gu = vec2<f32>(0.0);
    if (m.z >= 0.0) {
        let xy = m.xy / (1.0 + m.z);
        c = textureSampleLevel(t_front, s_dp, vec2<f32>(xy.x * 0.5 + 0.5, 0.5 - xy.y * 0.5), 0.0);
        gu = vec2<f32>(xy.x * 0.25 + 0.25, 0.5 - xy.y * 0.5);
    } else {
        let xy = vec2<f32>(-m.x, m.y) / (1.0 - m.z);
        c = textureSampleLevel(t_back, s_dp, vec2<f32>(xy.x * 0.5 + 0.5, 0.5 - xy.y * 0.5), 0.0);
        gu = vec2<f32>(xy.x * 0.25 + 0.75, 0.5 - xy.y * 0.5);
    }
    // (PS_Blit4x: the picture's texel twice over, added to what the map holds)
    let glow = textureSampleLevel(t_glow, s_dp, gu, 0.0).rgb * 2.0;
    if (c.a < 0.5) {
        // (nothing drawn there: the face keeps its sky - but for the garage's lights, which the
        // original adds over its cleared map)
        if (max(glow.r, max(glow.g, glow.b)) <= 0.0) { discard; }
        return vec4<f32>(glow, 1.0);
    }
    return vec4<f32>(c.rgb / c.a + glow, 1.0);
}
