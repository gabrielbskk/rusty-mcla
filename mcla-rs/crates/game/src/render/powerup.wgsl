// Step 19f: the power-up sources as the original draws them (pu_draw.rs).
//
// The card (sub_82681760): a quad facing the camera, 2 m_powerupWidth x 2 m_powerupHeight, its
// middle m_iconHeight above the source, drawn with powerup.fxc (effects/0x6050939b) technique
// "circle_draw", blend source alpha / inverse source alpha:
//   PS_TexturedBKG  rgb = sat(IconTex((uv - 0.5) x 1.5 + 0.5).r ^ 2), a = 0.85 x IconTexBkg(uv).a
//                   (IconTex = $/ui/powerups/hud_pu_<name>, IconTexBkg = hud_pu_bkgrnd_mask)
//   PS_Textured     d = uv - 0.5, r = |d|; ring = sat(100 (r - 0.3844)) sat(100 (0.49 - r));
//                   eight gaps: |d turned by seg0426 / seg3715| - 0.03, each x 1000 saturated;
//                   rgb = sat(10000 ring) x the vertex colour (m_powerupColor),
//                   a = sat(0.3 sat(100 (0.49 - r)) + ring)
// Both programs' alpha is taken by gInvColorExpBias (2^bias): read as 1 here.
// The two passes are put together in one program: the ring's pass over the icon's.
//
// The ground glow (sub_82682318): a fan of 33 points 0.1 above the source - m_groundGlowColor1
// in the middle, m_groundGlowColor2 round the edge at radius m_groundGlowScale (the edge's 32
// points i / 31 of a turn) - blend source alpha / one, both alphas x the distance fade
// clamp((1 - 0.004 d) x 2, 0, 1).

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
    sky_zenith: vec4<f32>,
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

// What the tone map (post.wgsl) will turn into this display colour (as particle.wgsl).
fn to_scene(d: vec3<f32>) -> vec3<f32> {
    let v = d / max(g.grade.rgb, vec3<f32>(0.001));
    let y = clamp(dot(v, vec3<f32>(0.2125, 0.7154, 0.0721)), 0.0, 0.985);
    let w = max(g.tone.y, 0.001);
    let l = 0.5 * w * (sqrt((1.0 - y) * (1.0 - y) + 4.0 * y / w) - (1.0 - y));
    return v * (l / max(y, 0.0001)) / max(g.tone.x, 0.001);
}

struct Cam {
    // the camera's right and up (the original's camera matrix +64 / +80)
    right: vec4<f32>,
    up: vec4<f32>,
    // seg0426 = (cos a, sin a, -sin a, cos a), seg3715 the same for a + pi / 4
    seg0426: vec4<f32>,
    seg3715: vec4<f32>,
};
@group(1) @binding(0) var t_icons: texture_2d_array<f32>;
@group(1) @binding(1) var s_icons: sampler;
@group(1) @binding(2) var<uniform> cam: Cam;

struct CardOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) @interpolate(flat) layer: i32,
};

@vertex
fn vs_card(@builtin(vertex_index) vi: u32, @location(0) centre_w: vec4<f32>, @location(1) color: vec4<f32>, @location(2) misc: vec4<f32>) -> CardOut {
    // two triangles: top left, top right, bottom left / bottom left, top right, bottom right
    var cx = array<f32, 6>(-1.0, 1.0, -1.0, -1.0, 1.0, 1.0);
    var cy = array<f32, 6>(1.0, 1.0, -1.0, -1.0, 1.0, -1.0);
    let x = cx[vi];
    let y = cy[vi];
    let p = centre_w.xyz + cam.right.xyz * (x * centre_w.w) + cam.up.xyz * (y * misc.y);
    var o: CardOut;
    o.clip = g.view_proj * vec4<f32>(p, 1.0);
    o.uv = vec2<f32>(0.5 * (x + 1.0), 0.5 * (1.0 - y));
    o.color = color;
    o.layer = i32(misc.x);
    return o;
}

@fragment
fn fs_card(in: CardOut) -> @location(0) vec4<f32> {
    // PS_TexturedBKG
    let iuv = (in.uv - vec2<f32>(0.5)) * 1.5 + vec2<f32>(0.5);
    var icon = 0.0;
    if (in.layer >= 0 && all(iuv >= vec2<f32>(0.0)) && all(iuv <= vec2<f32>(1.0))) {
        icon = textureSampleLevel(t_icons, s_icons, iuv, in.layer, 0.0).r;
    }
    let mask = textureSampleLevel(t_icons, s_icons, in.uv, 24, 0.0).a;
    let c0 = vec3<f32>(clamp(icon * icon, 0.0, 1.0));
    let a0 = 0.85 * mask;
    // PS_Textured
    let d = in.uv - vec2<f32>(0.5);
    let r = length(d);
    let s0 = cam.seg0426;
    let s1 = cam.seg3715;
    let q = vec4<f32>(d.x * s0.x + d.y * s0.y, d.x * s0.z + d.y * s0.w, d.x * s1.z + d.y * s1.w, d.x * s1.x + d.y * s1.y);
    let gaps = clamp((abs(q) - vec4<f32>(0.03)) * 1000.0, vec4<f32>(0.0), vec4<f32>(1.0));
    let outer = clamp(100.0 * (0.49 - r), 0.0, 1.0);
    let ring = clamp(100.0 * (r - 0.3844), 0.0, 1.0) * outer * gaps.x * gaps.y * gaps.z * gaps.w;
    let c1 = clamp(10000.0 * ring, 0.0, 1.0) * in.color.rgb;
    let a1 = clamp(0.3 * outer + ring, 0.0, 1.0);
    // the ring's pass over the icon's, premultiplied
    let rgb = to_scene(c1) * a1 + to_scene(c0) * a0 * (1.0 - a1);
    let a = 1.0 - (1.0 - a0) * (1.0 - a1);
    return vec4<f32>(rgb, a);
}

struct GlowOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_glow(@builtin(vertex_index) vi: u32, @location(0) pos_r: vec4<f32>, @location(1) c1: vec4<f32>, @location(2) c2: vec4<f32>) -> GlowOut {
    // 31 triangles: the middle, edge point k, edge point k + 1 (k = 0 .. 30)
    let tri = vi / 3u;
    let corner = vi % 3u;
    var o: GlowOut;
    var p = vec3<f32>(0.0, 0.1, 0.0);
    o.color = c1;
    if (corner != 0u) {
        let k = f32(tri + corner - 1u);
        let a = k * 0.032258064 * 6.2831855;
        p = vec3<f32>(cos(a) * pos_r.w, 0.1, sin(a) * pos_r.w);
        o.color = c2;
    }
    o.clip = g.view_proj * vec4<f32>(pos_r.xyz + p, 1.0);
    return o;
}

@fragment
fn fs_glow(in: GlowOut) -> @location(0) vec4<f32> {
    return vec4<f32>(to_scene(in.color.rgb), in.color.a);
}
