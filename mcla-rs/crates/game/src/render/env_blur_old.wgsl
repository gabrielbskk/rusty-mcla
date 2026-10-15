// The blurred copy of the cars' environment map (the original shrinks its 512 x 256 dual
// paraboloid map to 64 x 32 - 5.6 degrees a texel - and runs a Gaussian over it, PS_BlurHoriz /
// PS_BlurVert; the paint's soft specular reads it, dpSpecFrontSampler). Here: a 16 x 16 cube,
// each texel a Gaussian-weighted disc of 64 taps of the sharp cube, about 11 degrees wide.

@group(0) @binding(0) var t_env: texture_cube<f32>;
@group(0) @binding(1) var s_env: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) face: u32,
};

@vertex
fn vs_blur(@builtin(vertex_index) vi: u32, @builtin(instance_index) face: u32) -> VsOut {
    let p = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    var out: VsOut;
    out.pos = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    // (texture coordinates of the face, v down)
    out.uv = vec2<f32>(p.x, 1.0 - p.y);
    out.face = face;
    return out;
}

// the direction a cube map's sampler maps to this texel of this face
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
fn fs_blur(in: VsOut) -> @location(0) vec4<f32> {
    let d = normalize(face_dir(in.face, in.uv));
    let up = select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(d.y) > 0.9);
    let tx = normalize(cross(up, d));
    let bx = cross(d, tx);
    var sum = vec3<f32>(0.0);
    var wsum = 0.0;
    for (var i = 0; i < 64; i = i + 1) {
        // a disc of 25 degrees' radius, taps on a golden-angle spiral
        let q = (f32(i) + 0.5) / 64.0;
        let r = sqrt(q) * 0.4663;
        let a = f32(i) * 2.399963;
        let w = exp(-q * 2.6);
        // (the original's map holds no more than about 2.3 anywhere at noon - its sky has no
        // sun disc of the main sky's strength; the taps are held to that)
        sum = sum + min(textureSampleLevel(t_env, s_env, d + (tx * cos(a) + bx * sin(a)) * r, 0.0).rgb, vec3<f32>(2.3)) * w;
        wsum = wsum + w;
    }
    // (the original's two passes each weigh their taps 0.1 + 2 x (0.1 + 0.09 + ... + 0.01) = 1.2
    // in all - gaussCoefs - so the blurred map comes out 1.44 times the plain average)
    return vec4<f32>(sum / wsum * 1.44, 1.0);
}
