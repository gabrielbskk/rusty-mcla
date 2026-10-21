//! Post-processing chain, laid out as the original's (rage_postfx; see LIGHTING_WORK.md): the
//! scene is drawn into an HDR target; half and quarter size copies feed the log-average
//! luminance (128 -> 64 -> 16 -> 4 -> 1, adapted over time) and the bloom (bright pass and two
//! Gauss passes at quarter size); the tone map writes the display. Android refreshes exposure and
//! bloom every other frame but still tone-maps and draws current lamp glow each frame. The composite
//! also carries this port's speed-dependent radial blur.

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

/// The scene's colour target: linear light, values above 1 kept.
pub const SCENE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const LUM_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// What the tone map is tuned by: the weather's keys at the hour (`sky::Tod`).
#[derive(Clone, Copy, Debug)]
pub struct Tune {
    /// `m_Luminance`.x
    pub middle_grey: f32,
    /// `m_Luminance`.y squared
    pub white: f32,
    /// `m_BrightPass`: threshold, offset, white squared
    pub bright: [f32; 3],
    /// lower and upper limit of the adapted luminance
    pub limits: [f32; 2],
    /// adaption rate (Lambda)
    pub lambda: f32,
    pub saturation: f32,
    pub color_correct: [f32; 3],
    /// seconds since the last frame
    pub dt: f32,
    /// radial blur amount and its centre in uv
    pub radial: f32,
    pub center: [f32; 2],
    /// tone map only: no bloom, no blur
    pub plain: bool,
    /// test hook: this adapted luminance instead of the measured one
    pub force_lum: Option<f32>,
    /// `m_DoFParams` of the weather's keys (PSStreakMotionBlur's DofParams): the distance the far
    /// blur begins at (m), its ramp (m), its scale; scale 0: no depth of field
    pub dof: [f32; 4],
    /// the scene's inverse view-projection and the camera's place: the depth of field's distance
    pub inv_view_proj: [[f32; 4]; 4],
    pub cam_pos: [f32; 3],
}

impl Default for Tune {
    fn default() -> Self {
        // noon in the Nice weather
        Tune { middle_grey: 0.25, white: 0.21646, bright: [1.6203, 4.6213, 0.032782], limits: [0.15, 0.2], lambda: 3.7266, saturation: 0.9, color_correct: [0.5862, 0.53442, 0.43101], dt: 1.0 / 60.0, radial: 0.0, center: [0.5, 0.5], plain: false, force_lum: None, dof: [0.0; 4], inv_view_proj: [[0.0; 4]; 4], cam_pos: [0.0; 3] }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
struct Params {
    a: [f32; 4],
    b: [f32; 4],
    c: [f32; 4],
    d: [f32; 4],
    e: [f32; 4],
}

/// What the depth of field's pass is drawn with (post.wgsl `DofU`).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
struct DofU {
    inv_view_proj: [[f32; 4]; 4],
    cam: [f32; 4],
    dof: [f32; 4],
    px: [f32; 4],
}

/// The depth of field's buffer. The original's is 640 x 360 beside a 1280 x 720 frame: 360 rows at
/// any window size, like the bloom's 180.
fn dof_size(w: u32, h: u32) -> (u32, u32) {
    if h < 360 {
        return ((w / 2).max(1), (h / 2).max(1));
    }
    (((w as f32) * 360.0 / (h as f32)).round() as u32, 360)
}

/// MCLA_DEBUG_VIEW 500 .. 519 (the depth of field's): 500 off, 501 the weight, 502 the buffer
/// everywhere, 503 its blurred part through the 7e3 format once more, 504 the scene through it
/// too, 505 the taps as they fall without the half-pixel shift of the quads' coordinates, 506 the
/// sky takes part, 507 without the one sharp tap, 508 the tone map's tap on the pixel's centre.
fn dof_view() -> u32 {
    static V: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *V.get_or_init(|| std::env::var("MCLA_DEBUG_VIEW").ok().and_then(|v| v.trim().parse().ok()).filter(|v| (500..520).contains(v)).unwrap_or(0))
}

/// MCLA_DOF=0 (or view 500): the tone map reads the scene alone, as before.
fn dof_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("MCLA_DOF").map_or(true, |v| v.trim() != "0") && dof_view() != 500)
}

struct Target {
    tex: wgpu::Texture,
    view: wgpu::TextureView,
    w: u32,
    h: u32,
}

fn target(device: &wgpu::Device, w: u32, h: u32, format: wgpu::TextureFormat, label: &str) -> Target {
    let t = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    Target { view: t.create_view(&Default::default()), tex: t, w: w.max(1), h: h.max(1) }
}

/// The bloom's two targets. The original always draws 1280 x 720 and blooms at 320 x 180: its
/// nine Gauss taps reach +-4 texels = +-16 of 720 rows. A quarter of a larger window made the
/// halo narrower on screen (1600 x 900: 400 x 225, +-12.8 of 720 rows - measured on the car's
/// side: the halo's tails 0.006 against the original's 0.013 at 20 px). So: 180 rows at any
/// window size; the bright pass reads the quarter-size scene with one bilinear tap as before.
/// MCLA_BLOOM_WIN=1: a quarter of the window, as before.
fn bloom_size(w: u32, h: u32) -> (u32, u32) {
    if std::env::var("MCLA_BLOOM_WIN").is_ok() || h < 180 {
        return (w / 4, h / 4);
    }
    (((w as f32) * 180.0 / (h as f32)).round() as u32, 180)
}

const STEPS: usize = 12;

pub struct Post {
    scene: Target,
    half: Target,
    quarter: Target,
    bloom_a: Target,
    bloom_b: Target,
    /// the lamps' glow pass (halo.rs): display values the tone map adds to its encoded result
    halo: Target,
    /// the depth of field's buffer (PSStreakMotionBlur's target): rgb = the scene blurred, a = how far
    /// beyond `m_DoFParams`.x the pixel lies; the tone map lerps the scene towards it
    dof: Target,
    dof_layout: wgpu::BindGroupLayout,
    dof_buf: wgpu::Buffer,
    pipe_dof: wgpu::RenderPipeline,
    /// log luminance: 128, 64, 16, 4
    lum: [Target; 4],
    /// the adapted luminance of the frame before and of this one (they swap)
    adapted: [Target; 2],
    flip: std::cell::Cell<usize>,
    first: std::cell::Cell<bool>,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    pipe_scale: wgpu::RenderPipeline,
    pipe_lum_log: wgpu::RenderPipeline,
    pipe_lum_down: wgpu::RenderPipeline,
    pipe_lum_adapt: wgpu::RenderPipeline,
    pipe_bright: wgpu::RenderPipeline,
    pipe_gauss: wgpu::RenderPipeline,
    pipe_tonemap: wgpu::RenderPipeline,
    bufs: Vec<wgpu::Buffer>,
    /// [adapted target of this frame][step]
    groups: [Vec<wgpu::BindGroup>; 2],
    mobile_frame: std::cell::Cell<u64>,
    elapsed_since_chain: std::cell::Cell<f32>,
}

impl Post {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, w: u32, h: u32) -> Post {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("post"), source: wgpu::ShaderSource::Wgsl(include_str!("post.wgsl").into()) });
        let tex_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post layout"),
            entries: &[
                tex_entry(0),
                tex_entry(1),
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                tex_entry(4),
                tex_entry(5),
                tex_entry(6),
            ],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("post pl"), bind_group_layouts: &[&layout], push_constant_ranges: &[] });
        let make = |entry: &str, fmt: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&pl),
                vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_post"), buffers: &[], compilation_options: Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    targets: &[Some(wgpu::ColorTargetState { format: fmt, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                    compilation_options: Default::default(),
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview: None,
                cache: None,
            })
        };
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("post sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        // the depth of field's pass: its own bindings (the scene, the scene's depth)
        let dof_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("dof layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 10, ..tex_entry(0) },
                wgpu::BindGroupLayoutEntry {
                    binding: 11,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 12, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                wgpu::BindGroupLayoutEntry {
                    binding: 13,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
        });
        let dof_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("dof pl"), bind_group_layouts: &[&dof_layout], push_constant_ranges: &[] });
        let pipe_dof = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("fs_dof"),
            layout: Some(&dof_pl),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_post"), buffers: &[], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_dof"),
                targets: &[Some(wgpu::ColorTargetState { format: SCENE_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });
        let dof_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("dof params"), contents: bytemuck::bytes_of(&DofU::default()), usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST });
        let bufs = (0..STEPS).map(|_| device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("post params"), contents: bytemuck::bytes_of(&Params::default()), usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST })).collect();
        let lum = |n: u32, label: &str| target(device, n, n, LUM_FORMAT, label);
        let mut p = Post {
            scene: target(device, w, h, SCENE_FORMAT, "scene"),
            half: target(device, w / 2, h / 2, SCENE_FORMAT, "scene half"),
            quarter: target(device, w / 4, h / 4, SCENE_FORMAT, "scene quarter"),
            bloom_a: target(device, bloom_size(w, h).0, bloom_size(w, h).1, SCENE_FORMAT, "bloom a"),
            bloom_b: target(device, bloom_size(w, h).0, bloom_size(w, h).1, SCENE_FORMAT, "bloom b"),
            halo: target(device, w, h, SCENE_FORMAT, "halo"),
            dof: target(device, dof_size(w, h).0, dof_size(w, h).1, SCENE_FORMAT, "dof"),
            dof_layout,
            dof_buf,
            pipe_dof,
            lum: [lum(128, "lum 128"), lum(64, "lum 64"), lum(16, "lum 16"), lum(4, "lum 4")],
            adapted: [lum(1, "adapted a"), lum(1, "adapted b")],
            flip: std::cell::Cell::new(0),
            first: std::cell::Cell::new(true),
            pipe_scale: make("fs_scale", SCENE_FORMAT),
            pipe_lum_log: make("fs_lum_log", LUM_FORMAT),
            pipe_lum_down: make("fs_lum_down", LUM_FORMAT),
            pipe_lum_adapt: make("fs_lum_adapt", LUM_FORMAT),
            pipe_bright: make("fs_bright", SCENE_FORMAT),
            pipe_gauss: make("fs_gauss", SCENE_FORMAT),
            pipe_tonemap: make("fs_tonemap", format),
            layout,
            sampler,
            bufs,
            groups: [Vec::new(), Vec::new()],
            mobile_frame: std::cell::Cell::new(0),
            elapsed_since_chain: std::cell::Cell::new(0.0),
        };
        p.rebuild_groups(device);
        p
    }

    fn rebuild_groups(&mut self, device: &wgpu::Device) {
        let mk = |a: &wgpu::TextureView, b: &wgpu::TextureView, c: &wgpu::TextureView, buf: &wgpu::Buffer| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("post bg"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(a) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(b) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                    wgpu::BindGroupEntry { binding: 3, resource: buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(c) },
                    wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&self.halo.view) },
                    wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(&self.dof.view) },
                ],
            })
        };
        let mut sets = Vec::new();
        for now in 0..2 {
            let (new, old) = (&self.adapted[now].view, &self.adapted[1 - now].view);
            let s = &self.scene.view;
            sets.push(vec![
                // 0: scene -> half
                mk(s, s, s, &self.bufs[0]),
                // 1: half -> quarter
                mk(&self.half.view, s, s, &self.bufs[1]),
                // 2: quarter -> log luminance 128
                mk(&self.quarter.view, s, s, &self.bufs[2]),
                // 3..5: 128 -> 64 -> 16 -> 4
                mk(&self.lum[0].view, s, s, &self.bufs[3]),
                mk(&self.lum[1].view, s, s, &self.bufs[4]),
                mk(&self.lum[2].view, s, s, &self.bufs[5]),
                // 6: 4 -> the adapted luminance (with the one of the frame before)
                mk(&self.lum[3].view, old, s, &self.bufs[6]),
                // 7: bright pass, quarter -> bloom a
                mk(&self.quarter.view, new, s, &self.bufs[7]),
                // 8, 9: Gauss, a -> b -> a
                mk(&self.bloom_a.view, s, s, &self.bufs[8]),
                mk(&self.bloom_b.view, s, s, &self.bufs[9]),
                // 10: tone map, scene + adapted luminance + bloom -> the display
                mk(s, new, &self.bloom_a.view, &self.bufs[10]),
            ]);
        }
        let second = sets.pop().unwrap();
        let first = sets.pop().unwrap();
        self.groups = [first, second];
    }

    pub fn resize(&mut self, device: &wgpu::Device, w: u32, h: u32) {
        self.scene = target(device, w, h, SCENE_FORMAT, "scene");
        self.half = target(device, w / 2, h / 2, SCENE_FORMAT, "scene half");
        self.quarter = target(device, w / 4, h / 4, SCENE_FORMAT, "scene quarter");
        self.bloom_a = target(device, bloom_size(w, h).0, bloom_size(w, h).1, SCENE_FORMAT, "bloom a");
        self.bloom_b = target(device, bloom_size(w, h).0, bloom_size(w, h).1, SCENE_FORMAT, "bloom b");
        self.halo = target(device, w, h, SCENE_FORMAT, "halo");
        self.dof = target(device, dof_size(w, h).0, dof_size(w, h).1, SCENE_FORMAT, "dof");
        self.rebuild_groups(device);
        self.flip.set(0);
        self.first.set(true);
        self.mobile_frame.set(0);
        self.elapsed_since_chain.set(0.0);
    }

    /// Diagnostics (MCLA_POST_DUMP=<file>): the bright pass's input (the quarter-size scene) and
    /// the finished bloom, raw - "MPS2", then per picture width, height and RGBA half floats.
    pub fn dump(&self, device: &wgpu::Device, queue: &wgpu::Queue, path: &str) {
        let mut out: Vec<u8> = Vec::new();
        out.extend_from_slice(b"MPS2");
        for t in [&self.quarter, &self.bloom_a] {
            out.extend_from_slice(&t.w.to_le_bytes());
            out.extend_from_slice(&t.h.to_le_bytes());
            let bpr = (t.w * 8 + 255) / 256 * 256;
            let buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("post dump"), size: (bpr * t.h) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
            let mut enc = device.create_command_encoder(&Default::default());
            enc.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo { texture: &t.tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bpr), rows_per_image: Some(t.h) } },
                wgpu::Extent3d { width: t.w, height: t.h, depth_or_array_layers: 1 },
            );
            queue.submit([enc.finish()]);
            let slice = buf.slice(..);
            let (tx, rx) = std::sync::mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |r| {
                let _ = tx.send(r);
            });
            device.poll(wgpu::Maintain::Wait);
            if let Ok(Ok(())) = rx.recv() {
                let data = slice.get_mapped_range();
                for y in 0..t.h {
                    out.extend_from_slice(&data[(y * bpr) as usize..(y * bpr + t.w * 8) as usize]);
                }
            }
            buf.unmap();
        }
        if let Err(e) = std::fs::write(path, &out) {
            log::warn!("post dump {}: {}", path, e);
        }
    }

    /// The quarter-size copy of the scene (PSScaleBuffer's output: after fog and glows, before
    /// bloom and the tone map). Until this frame's chain runs it holds the frame before - what
    /// the ground sheet shows (ground_sheet.rs).
    pub fn quarter_view(&self) -> &wgpu::TextureView {
        &self.quarter.view
    }

    /// The picture of the lamps' glow pass (halo.rs draws it every frame, the tone map adds it).
    pub fn halo_view(&self) -> &wgpu::TextureView {
        &self.halo.view
    }

    /// The off-screen colour target the scene is drawn into.
    pub fn scene_view(&self) -> &wgpu::TextureView {
        &self.scene.view
    }

    /// Run the chain and write the result to `out`. `depth`: the scene's depth buffer (the depth of
    /// field's distance).
    pub fn run(&self, device: &wgpu::Device, queue: &wgpu::Queue, enc: &mut wgpu::CommandEncoder, out: &wgpu::TextureView, depth: &wgpu::TextureView, t: &Tune) {
        self.elapsed_since_chain.set((self.elapsed_since_chain.get() + t.dt.max(0.0)).min(0.5));
        let first_pending = self.first.get();
        let frame_no = self.mobile_frame.get();
        let update_chain = !cfg!(target_os = "android") || first_pending || frame_no % 2 == 0;
        self.mobile_frame.set(frame_no.wrapping_add(1));
        let first = first_pending && update_chain;
        let mut now = self.flip.get();
        if update_chain {
            if !first { now = 1 - now; }
            self.flip.set(now);
            self.first.set(false);
        }
        let aspect = self.scene.w as f32 / self.scene.h as f32;
        let texel = |t: &Target| [1.0 / t.w as f32, 1.0 / t.h as f32, 0.0, 0.0];
        // The depth of field (PSStreakMotionBlur + PSToneMap instr. 6 - 9, LIGHTING_WORK.md): not under
        // --no-post, not without the weather's key.
        let dof_on = !cfg!(target_os = "android") && t.dof[2] > 0.0 && !t.plain && dof_enabled();
        let grade = Params {
            a: [t.middle_grey, t.bright[0], t.bright[1], t.bright[2]],
            b: [t.limits[0], t.limits[1], t.white, t.saturation],
            c: [t.color_correct[0], t.color_correct[1], t.color_correct[2], t.radial],
            d: [t.center[0], t.center[1], aspect, if let Some(l) = t.force_lum { 2.0 + l.max(0.0) } else if t.plain { 1.0 } else { 0.0 }],
            e: [if dof_on { 1.0 } else { 0.0 }, dof_view() as f32, 0.5 / self.dof.w as f32, 0.5 / self.dof.h as f32],
        };
        let only = |a: [f32; 4]| Params { a, ..Default::default() };
        let mut ps = [
            only([0.0; 4]),
            only([0.0; 4]),
            only(texel(&self.quarter)),
            only(texel(&self.lum[0])),
            only(texel(&self.lum[1])),
            only(texel(&self.lum[2])),
            only([1.0 - (-self.elapsed_since_chain.get() * t.lambda).exp(), if first { 1.0 } else { 0.0 }, 0.0, 0.0]),
            grade,
            only([1.0 / self.bloom_a.w as f32, 0.0, 0.0, 0.0]),
            only([0.0, 1.0 / self.bloom_b.h as f32, 0.0, 0.0]),
            grade,
        ];
        // (the 4 -> 1 step reads its 16 taps from the 4x4 target, a texel apart)
        ps[6].b = texel(&self.lum[3]);
        if update_chain {
            for (b, p) in self.bufs.iter().zip(ps.iter()) {
                queue.write_buffer(b, 0, bytemuck::bytes_of(p));
            }
            self.elapsed_since_chain.set(0.0);
        } else {
            // Exposure and bloom stay resident from the preceding frame. Refresh only the
            // per-frame tone-map controls and current lamp glow on this Android frame.
            queue.write_buffer(&self.bufs[10], 0, bytemuck::bytes_of(&ps[10]));
        }
        let steps: [(&wgpu::TextureView, &wgpu::RenderPipeline, usize); 11] = [
            (&self.half.view, &self.pipe_scale, 0),
            (&self.quarter.view, &self.pipe_scale, 1),
            (&self.lum[0].view, &self.pipe_lum_log, 2),
            (&self.lum[1].view, &self.pipe_lum_down, 3),
            (&self.lum[2].view, &self.pipe_lum_down, 4),
            (&self.lum[3].view, &self.pipe_lum_down, 5),
            (&self.adapted[now].view, &self.pipe_lum_adapt, 6),
            (&self.bloom_a.view, &self.pipe_bright, 7),
            (&self.bloom_b.view, &self.pipe_gauss, 8),
            (&self.bloom_a.view, &self.pipe_gauss, 9),
            (out, &self.pipe_tonemap, 10),
        ];
        if update_chain && dof_on {
            let u = DofU {
                inv_view_proj: t.inv_view_proj,
                cam: [t.cam_pos[0], t.cam_pos[1], t.cam_pos[2], dof_view() as f32],
                dof: t.dof,
                // one pixel of the original's 1280 x 720 frame in uv (half a texel of the buffer)
                px: [0.5 / self.dof.w as f32, 0.5 / self.dof.h as f32, 0.0, 0.0],
            };
            queue.write_buffer(&self.dof_buf, 0, bytemuck::bytes_of(&u));
            let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("dof bg"),
                layout: &self.dof_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 10, resource: wgpu::BindingResource::TextureView(&self.scene.view) },
                    wgpu::BindGroupEntry { binding: 11, resource: wgpu::BindingResource::TextureView(depth) },
                    wgpu::BindGroupEntry { binding: 12, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                    wgpu::BindGroupEntry { binding: 13, resource: self.dof_buf.as_entire_binding() },
                ],
            });
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("dof"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &self.dof.view, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.pipe_dof);
            pass.set_bind_group(0, &bg, &[]);
            pass.draw(0..3, 0..1);
        }
        for (step, (view, pipe, group)) in steps.iter().enumerate() {
            if !update_chain && step != 10 { continue; }
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("post"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(pipe);
            pass.set_bind_group(0, &self.groups[now][*group], &[]);
            pass.draw(0..3, 0..1);
        }
    }
}
