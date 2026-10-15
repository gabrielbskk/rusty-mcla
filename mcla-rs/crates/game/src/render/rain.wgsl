// Rain as the original draws it (LIGHTING_WORK.md, "Rain as the original draws it"):
//   - the drops and their splashes: the effect `rain_system` (ParticleRenderVS / ParticleRenderPS
//     of shaders/effects/fxl_final/0x9e307cae), one quad a particle;
//   - the street lamps' light cones in the rain: `rage_spotlightfog` (vs_main_precal /
//     ps_mainFast), one cone a lamp.
// Both are blended source alpha / one minus source alpha over the finished scene, depth tested,
// no depth write.

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

// the effect's constants, under their own names
struct RainU {
    global_light: vec4<f32>,   // GlobalLightColor: m_KeyColor + m_KeyAmbLight + RainAmbient
    scale_offset: vec4<f32>,   // ScaleOffset: splash opacity, Rainstrength, SplashSizeScale, rain opacity
    tune: vec4<f32>,           // TuneParams: SplashOffset, Rainthreshold, Streakthickness, Streaklength
    spot_pos: vec4<f32>,       // SpotLightPos
    spot_dir: vec4<f32>,       // SpotLightDir
    spot_params: vec4<f32>,    // SpotLightParams: strength, height stretch
    spot_color: vec4<f32>,     // SpotLightColor: RainSpotLight
    fade_sphere: vec4<f32>,    // FadeOutSphere: centre, radius
    lights: array<vec4<f32>, 4>,      // PointLights: position, reach
    light_cols: array<vec4<f32>, 4>,  // PointLightColors
};
@group(1) @binding(0) var t_rain: texture_2d<f32>;
@group(1) @binding(1) var s_rain: sampler;
@group(1) @binding(2) var t_anim: texture_2d<f32>;
@group(1) @binding(3) var<uniform> r: RainU;

struct DropOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) light: vec3<f32>,
    @location(2) fade: vec2<f32>,   // alpha, 1 = splash
};

@vertex
fn vs_drop(
    @builtin(vertex_index) vi: u32,
    @location(0) pos_state: vec4<f32>,   // position; state: 1.25 falling, 0.75 .. 0 the splash's four frames, below 0 dead
    @location(1) vel: vec4<f32>,
) -> DropOut {
    var o: DropOut;
    let u = f32(vi & 1u);
    let v = f32((vi >> 1u) & 1u);
    let p = pos_state.xyz;
    let state = pos_state.w;
    let to_cam = g.cam_pos.xyz - p;
    let dist = max(length(to_cam), 1e-4);
    let d = to_cam / dist;
    var wp = p;
    var alpha = step(0.0, state);
    if (state < 1.0) {
        // a splash: a card facing the camera round the vertical, SplashSizeScale wide, half
        // as high (flatter the steeper the view), smaller nearer than 13 m
        let hd = max(length(to_cam.xz), 1e-4);
        let side = vec3<f32>(to_cam.z / hd, 0.0, -to_cam.x / hd);
        let k = min(dist * 0.0769230798, 1.0);
        let s = r.scale_offset.z;
        wp = p + (side * (u * s) + vec3<f32>(0.0, -0.5 * v * s * length(d.xz), 0.0)) * k;
        o.uv = vec2<f32>(0.25 * u + 0.75 - state, v);
        o.fade.y = 1.0;
        alpha = alpha * r.scale_offset.x;
    } else {
        // a streak: from the drop back along its velocity (x Streaklength), a constant width on
        // screen (Streakthickness x distance / 91)
        let c = cross(to_cam, vel.xyz);
        let cl = length(c);
        if (cl < 1e-6) {
            alpha = 0.0;
        }
        let side = c / max(cl, 1e-6);
        wp = p + side * (u * r.tune.z * 0.0109890122 * dist) - vel.xyz * (v * r.tune.w);
        o.uv = vec2<f32>(1.0 - u, 1.0 - v);
        o.fade.y = 0.0;
        alpha = alpha * r.scale_offset.w;
    }
    // the light on it: one colour for all of it, four point lights, one "spot"
    var light = r.global_light.rgb;
    for (var i = 0; i < 4; i = i + 1) {
        let l = r.lights[i].xyz - wp;
        let a = clamp(r.lights[i].w * inverseSqrt(max(dot(l, l), 1e-8)), 0.0, 1.0);
        light = light + r.light_cols[i].rgb * (a * a);
    }
    let sl = (r.spot_pos.xyz - wp) * vec3<f32>(1.0, r.spot_params.y, 1.0);
    let sll = max(length(sl), 1e-4);
    let sd = dot(sl / sll, r.spot_dir.xyz);
    if (sd > 0.0) {
        // (the ninth power as exp(9 log): far beyond the clamp for anything but a unit vector)
        let e = min(9.0 * log(max(sd, 1e-20)), 40.0);
        light = light + r.spot_color.rgb * clamp(exp(e) * r.spot_params.x / sll, 0.0, 2.0);
    }
    // nothing inside the sphere round the car (half as flat again in height)
    let fs = (wp - r.fade_sphere.xyz) * vec3<f32>(1.0, 0.5, 1.0);
    if (length(fs) < r.fade_sphere.w) {
        alpha = 0.0;
    }
    o.light = light;
    o.fade.x = alpha;
    o.clip = g.view_proj * vec4<f32>(wp, 1.0);
    return o;
}

@fragment
fn fs_drop(in: DropOut) -> @location(0) vec4<f32> {
    let a = textureSample(t_rain, s_rain, in.uv).r;
    let b = textureSample(t_anim, s_rain, in.uv).r;
    return vec4<f32>(in.light, in.fade.x * mix(a, b, step(0.5, in.fade.y)));
}

// ---- the lamps' cones ------------------------------------------------------------------

const CONE_SEGS: u32 = 24u;

struct ConeOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) @interpolate(flat) apex_h: vec4<f32>,
    @location(2) @interpolate(flat) dir_tan: vec4<f32>,
    @location(3) @interpolate(flat) color_fade: vec4<f32>,
    @location(4) @interpolate(flat) lamp: vec4<f32>,
};

@vertex
fn vs_cone(
    @builtin(vertex_index) vi: u32,
    @location(0) apex_h: vec4<f32>,      // the lamp, the cone's height
    @location(1) dir_tan: vec4<f32>,     // the way it shines, radius / height
    @location(2) color_fade: vec4<f32>,  // LightColor rgb, LightVolume.y (the lamp's fade)
    @location(3) lamp: vec4<f32>,        // LightColor.w, cone scale, cone bias, LightVolume.x
) -> ConeOut {
    var o: ConeOut;
    let tri = vi / 3u;
    let k = vi % 3u;
    let a = normalize(dir_tan.xyz);
    var ex = cross(a, vec3<f32>(1.0, 0.0, 0.0));
    if (dot(ex, ex) < 1e-4) {
        ex = cross(a, vec3<f32>(0.0, 0.0, 1.0));
    }
    ex = normalize(ex);
    let ez = cross(a, ex);
    var wp = apex_h.xyz;
    if (k != 0u) {
        let step_a = 6.28318531 / f32(CONE_SEGS);
        let ang = f32(tri + k - 1u) * step_a;
        // (the flat sides stand outside the round cone)
        let rad = apex_h.w * dir_tan.w / cos(0.5 * step_a);
        wp = apex_h.xyz + a * apex_h.w + (ex * cos(ang) + ez * sin(ang)) * rad;
    }
    o.world = wp;
    o.apex_h = apex_h;
    o.dir_tan = vec4<f32>(a, dir_tan.w);
    o.color_fade = color_fade;
    o.lamp = lamp;
    o.clip = g.view_proj * vec4<f32>(wp, 1.0);
    return o;
}

// One sample of the light inside the cone: min(cone ^ 1.4 x sat(1 / d^2) x LightColor.w, 0.3) x density.
fn cone_sample(p: vec3<f32>, apex: vec3<f32>, a: vec3<f32>, lamp: vec4<f32>) -> f32 {
    let l = p - apex;
    let d2 = max(dot(l, l), 1e-6);
    let c = dot(l, a) * inverseSqrt(d2) * lamp.y + lamp.z;
    let spot = clamp(pow(abs(c), 1.4), 0.0, 1.0);
    return min(spot * clamp(1.0 / d2, 0.0, 1.0) * lamp.x, 0.3) * lamp.w;
}

@fragment
fn fs_cone(in: ConeOut) -> @location(0) vec4<f32> {
    let cam = g.cam_pos.xyz;
    let apex = in.apex_h.xyz;
    let h_max = in.apex_h.w;
    let a = in.dir_tan.xyz;
    let ray = in.world - cam;
    let t_frag = length(ray);
    let v = ray / max(t_frag, 1e-4);
    // the ray against the cone |q|^2 = (1 + tan^2) (q . a)^2, q from the lamp
    let co = cam - apex;
    let kk = 1.0 + in.dir_tan.w * in.dir_tan.w;
    let ca = dot(co, a);
    let va = dot(v, a);
    let qa = 1.0 - kk * va * va;
    let qb = 2.0 * (dot(co, v) - kk * ca * va);
    let qc = dot(co, co) - kk * ca * ca;
    let disc = qb * qb - 4.0 * qa * qc;
    if (disc <= 0.0 || abs(qa) < 1e-6) {
        discard;
    }
    let sq = sqrt(disc);
    let r1 = (-qb - sq) / (2.0 * qa);
    let r2 = (-qb + sq) / (2.0 * qa);
    let t1 = min(r1, r2);
    let t2 = max(r1, r2);
    // the stretch of the ray inside the half of the cone below the lamp
    var t_in = t1;
    var t_out = t2;
    if (qa < 0.0) {
        if (va > 0.0) {
            t_in = t2;
            t_out = 1e9;
        } else {
            t_in = -1e9;
            t_out = t1;
        }
    } else if (ca + va * 0.5 * (t1 + t2) <= 0.0) {
        discard;
    }
    // ... and above its base
    var through_base = false;
    if (va > 1e-6) {
        t_out = min(t_out, (h_max - ca) / va);
    } else if (va < -1e-6) {
        let tb = (h_max - ca) / va;
        if (tb > t_in) {
            t_in = tb;
            through_base = true;
        }
    } else if (ca > h_max) {
        discard;
    }
    // the original draws the cone's outer faces towards the camera: nothing from inside it,
    // nothing through the base, and each ray once (the face nearer its way in)
    if (through_base || t_in <= 0.0 || t_out <= t_in || abs(t_frag - t_in) > abs(t_frag - t_out)) {
        discard;
    }
    let chord = t_out - t_in;
    let e = cam + v * t_in;
    let y = clamp(dot(e - apex, a) / h_max, 0.0, 1.0);
    let s0 = cone_sample(e + v * (chord * 0.125), apex, a, in.lamp);
    let s1 = cone_sample(e + v * (chord * 0.375), apex, a, in.lamp);
    let s2 = cone_sample(e + v * (chord * 0.625), apex, a, in.lamp);
    let s3 = cone_sample(e + v * (chord * 0.875), apex, a, in.lamp);
    let q = 1.0 - in.lamp.w;
    let sum = s0 + q * (s1 + q * (s2 + q * s3));
    // (faded out over the last eighth towards the base, and by the lamp's own fade)
    let alpha = clamp(sum * in.color_fade.w * clamp(8.0 * (1.0 - y), 0.0, 1.0), 0.0, 1.0);
    return vec4<f32>(in.color_fade.rgb, alpha);
}
