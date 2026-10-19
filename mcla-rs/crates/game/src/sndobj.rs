//! The original's sound objects at run time: a sound is started by its `sounds.dat` name and played as the game's
//! audio engine plays it (the tree of Wrapper / Multitrack / Randomized / Simple ... sounds, each level's volume,
//! pitch, pan, predelay, start offset, attack and release, the category's volume, the rolloff curve, the stereo pan
//! law and Doppler). Rules and sources: `docs/reference/audio/SOUND_FORMAT.md`.
//!
//! Approximate (written down there): TwinLoop and Crossfade play their first child; MathOperation / variables are not
//! evaluated; the "requested settings" the game passes are the subset in [`Req`].

use crate::audio::{Audio, Clip};
use glam::Vec3;
use hashbrown::HashMap;
use mcla_formats::audcfg::{self, db2lin, lin2db, Category, Curve, Dat, Payload, SoundHeader};
use mcla_formats::hash::joaat;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A wave of a converted bank: its file and the wave info the converter kept.
#[derive(Clone)]
struct WaveInfo {
    file: PathBuf,
    loop_start: i32,
    /// 0.01 dB
    headroom: i16,
}

/// What the game asks of a playing sound (the original's requested-settings slot, the part the port uses).
#[derive(Clone, Copy, Debug)]
pub struct Req {
    /// world position (positional sounds); `None` = at the listener, no rolloff
    pub pos: Option<Vec3>,
    /// dB added at the top (entry+64)
    pub volume: f32,
    /// cents added at the top (entry+80)
    pub pitch: i32,
    /// pan in degrees (entry+82), -1 = none
    pub pan: i16,
    /// Doppler factor (entry+84)
    pub doppler: f32,
    /// one of the player vehicle's sounds (the mixer's time-slow applies)
    pub player: bool,
}

impl Default for Req {
    fn default() -> Self {
        Req { pos: None, volume: 0.0, pitch: 0, pan: -1, doppler: 1.0, player: false }
    }
}

impl Req {
    pub fn at(pos: Vec3) -> Req {
        Req { pos: Some(pos), ..Default::default() }
    }
    pub fn vol(mut self, db: f32) -> Req {
        self.volume = db;
        self
    }
    pub fn pitch(mut self, cents: i32) -> Req {
        self.pitch = cents;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Handle(u64);

/// The original's random generator: multiply-with-carry, x = lo * 0x5CDCFAA7 + carry (sub_82145020 / sub_82145080).
struct Mwc {
    lo: u32,
    carry: u32,
}

impl Mwc {
    fn next(&mut self) -> u32 {
        let x = self.lo as u64 * 0x5CDC_FAA7 + self.carry as u64;
        self.lo = x as u32;
        self.carry = (x >> 32) as u32;
        self.lo
    }
    /// integer in [a, b]
    fn int(&mut self, a: i32, b: i32) -> i32 {
        if b <= a {
            return a;
        }
        ((self.next() & 0x7FFF_FFFF) % (b - a + 1) as u32) as i32 + a
    }
    /// float in [a, b)
    fn float(&mut self, a: f32, b: f32) -> f32 {
        (self.next() & 0x7F_FFFF) as f32 * (1.0 / 8_388_608.0) * (b - a) + a
    }
}

/// Settings handed down the tree each update (sub_8213BC50 / sub_8213B0F0).
#[derive(Clone, Copy)]
struct Ctx {
    vol: f32,
    pitch: i32,
    pan: i16,
    category: u32,
    curve: u32,
    curve_scale: f32,
    doppler: f32,
    mask: u8,
    /// tristate 1 (positional) as inherited: 0 off, 1 on, 2 unset
    positional: u8,
}

#[derive(PartialEq, Clone, Copy, Debug)]
enum St {
    Waiting,
    Playing,
    Done,
}

enum Kind {
    Wave { wave: Option<WaveInfo>, clip: Option<Arc<Clip>>, voice: u64, pos_ms: f32, dur_ms: f32, looping: bool },
    Stream { clip: Option<Arc<Clip>>, headroom: f32, voice: u64, pos_ms: f32, dur_ms: f32, looping: bool },
    /// Wrapper, Multitrack, Streaming fallback, OnStop's child, approximations
    Group { children: Vec<Node> },
    Looping { child: Box<Node>, child_hash: u32, count: i16, restart_ms: u16, plays: i32 },
    Sequential { list: Vec<u32>, at: usize, cur: Option<Box<Node>> },
    Retrig { child_hash: u32, count: i16, delay_ms: f32, next_ms: f32, made: i32, live: Vec<Node> },
    Envelope { a: f32, d: f32, s: f32, hold: f32, r: f32, curves: [u32; 3], child: Box<Node>, rel_from: Option<f32> },
    OnStop { child: Box<Node>, on_stop: u32, stopped: Option<Box<Node>> },
    Empty,
}

struct Node {
    h: SoundHeader,
    vol: f32,
    pitch: i32,
    pan: i16,
    predelay_ms: f32,
    start_ms: f32,
    st: St,
    /// ms since the node was made / since it started playing
    t: f32,
    play_t: f32,
    /// release: (start, end) in `t` ms
    release: Option<(f32, f32)>,
    kind: Kind,
}

struct Instance {
    id: u64,
    root: Node,
    req: Req,
    prev_dist: Option<f32>,
    doppler: f32,
    stop: bool,
}

pub struct SoundLib {
    root: PathBuf,
    snd: Dat,
    /// game.dat: the game objects (car audio profiles, cross-fader tables, districts ...)
    pub game: Dat,
    curves: HashMap<u32, Curve>,
    cats: HashMap<u32, Category>,
    /// bank name hash -> the bank's folder below the audio root (`vehicles/vehicle_aux`)
    bank_dir: HashMap<u32, String>,
    waves: HashMap<u32, Arc<HashMap<u32, WaveInfo>>>,
    /// stream channel hash -> (converted .mp3, the stream's lowest channel headroom)
    stream_of: Option<HashMap<u32, (PathBuf, i16)>>,
    clips: HashMap<PathBuf, Option<Arc<Clip>>>,
    rng: Mwc,
    /// Randomized objects' history (kept per object, as the original writes it into the metadata)
    history: HashMap<u32, (usize, Vec<u8>)>,
    insts: Vec<Instance>,
    next: u64,
    next_voice: u64,
    pub listener: (Vec3, Vec3, Vec3),
    /// the port's output reference: the original's MUSIC category level is the port's music at its default
    /// slider (0.35), so the balance between music and the sound objects is the original's (approximate:
    /// the original's absolute output level and its volume sliders are not decoded)
    pub out_gain: f32,
}

/// Voice id bit for the player vehicle's sounds: the mixer's time-slow (Zone, race end) applies to them.
pub const SLOWED: u64 = 1 << 40;

const VOICE_BASE: u64 = 0x4000_0000;
const NONE: u32 = 0xFFFF_FFFF;

impl SoundLib {
    /// `root` = the converted audio (`audio_pc`), `cfg` = `audio/X360/config` of the extracted cache.
    pub fn load(root: &Path, cfg: &Path) -> Option<SoundLib> {
        let snd = Dat::load(&cfg.join("sounds.dat"))?;
        let crv = Dat::load(&cfg.join("curves.dat"))?;
        let cat = Dat::load(&cfg.join("categories.dat"))?;
        let game = Dat::load(&cfg.join("game.dat"))?;
        let curves = crv.objects.iter().filter_map(|(n, _, _)| crv.get_name(n).map(|o| (joaat(n), Curve::parse(o)))).collect();
        let cats: HashMap<u32, Category> = audcfg::resolve_categories(&cat).into_iter().collect();
        let out_gain = 0.35 / db2lin(cats.get(&joaat("MUSIC")).map_or(0.0, |c| c.volume));
        // bank folders: each part of the bank's path by name, or by its hash where the archive lost the name
        let mut bank_dir = HashMap::new();
        for b in &snd.banks {
            let mut dir = PathBuf::new();
            let mut rel = Vec::new();
            for part in b.split(['\\', '/']) {
                let low = part.to_ascii_lowercase();
                let hashed = format!("0x{:08x}", joaat(part));
                let pick = if root.join(&dir).join(&low).exists() { low } else { hashed };
                dir.push(&pick);
                rel.push(pick);
            }
            bank_dir.insert(joaat(b), rel.join("/"));
        }
        log::info!("audio: sounds.dat {} sounds, {} curves, {} categories, {} banks", snd.len(), crv.len(), cat.len(), snd.banks.len());
        Some(SoundLib {
            root: root.to_path_buf(),
            snd,
            game,
            curves,
            cats,
            bank_dir,
            waves: HashMap::new(),
            stream_of: None,
            clips: HashMap::new(),
            rng: Mwc { lo: 0x1234_5678, carry: 0x9ABC },
            history: HashMap::new(),
            insts: Vec::new(),
            next: 1,
            next_voice: 0,
            listener: (Vec3::ZERO, Vec3::X, Vec3::NEG_Z),
            out_gain,
        })
    }

    pub fn exists(&self, name: &str) -> bool {
        self.snd.contains(joaat(name))
    }
    pub fn exists_hash(&self, h: u32) -> bool {
        self.snd.contains(h)
    }
    pub fn name_of(&self, h: u32) -> Option<&str> {
        self.snd.name_of(h)
    }
    /// A curve of curves.dat by name hash.
    pub fn curve(&self, h: u32) -> Option<&Curve> {
        self.curves.get(&h)
    }
    pub fn category(&self, name: &str) -> Option<Category> {
        self.cats.get(&joaat(name)).copied()
    }
    pub fn rand_float(&mut self, a: f32, b: f32) -> f32 {
        self.rng.float(a, b)
    }
    pub fn rand_int(&mut self, a: i32, b: i32) -> i32 {
        self.rng.int(a, b)
    }
    /// A curve's value by name hash; `None` when curves.dat has no such curve.
    pub fn curve_eval(&self, h: u32, x: f32) -> Option<f32> {
        self.curves.get(&h).map(|c| c.eval(x))
    }

    fn bank_waves(&mut self, bank: u32) -> Option<Arc<HashMap<u32, WaveInfo>>> {
        if let Some(w) = self.waves.get(&bank) {
            return Some(w.clone());
        }
        let dir = self.bank_dir.get(&bank)?.clone();
        let mut m = HashMap::new();
        if let Ok(t) = std::fs::read_to_string(self.root.join(&dir).join("_index.tsv")) {
            for l in t.lines() {
                let c: Vec<&str> = l.split('\t').collect();
                let (Some(name), Some(h)) = (c.first(), c.get(1).and_then(|h| u32::from_str_radix(h, 16).ok())) else { continue };
                let loop_start = c.get(5).and_then(|v| v.parse().ok()).unwrap_or(-1);
                let headroom = c.get(6).and_then(|v| v.parse().ok()).unwrap_or(0);
                m.insert(h, WaveInfo { file: self.root.join(&dir).join(format!("{name}.wav")), loop_start, headroom });
            }
        }
        let a = Arc::new(m);
        self.waves.insert(bank, a.clone());
        Some(a)
    }

    fn stream_index(&mut self) -> &HashMap<u32, (PathBuf, i16)> {
        if self.stream_of.is_none() {
            let mut m = HashMap::new();
            fn walk(dir: &Path, m: &mut HashMap<u32, (PathBuf, i16)>) {
                let Ok(rd) = std::fs::read_dir(dir) else { return };
                for e in rd.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        walk(&p, m);
                    } else if let Some(mp3) = p.to_str().and_then(|s| s.strip_suffix(".txt")).filter(|s| s.ends_with(".mp3")) {
                        let Ok(t) = std::fs::read_to_string(&p) else { continue };
                        let rows: Vec<(u32, i16)> = t.lines().filter_map(|l| {
                            let c: Vec<&str> = l.split('\t').collect();
                            Some((u32::from_str_radix(c.get(1)?, 16).ok()?, c.get(5).and_then(|v| v.parse().ok()).unwrap_or(0)))
                        }).collect();
                        let h0 = rows.iter().map(|r| r.1).min().unwrap_or(0);
                        for (h, _) in rows {
                            m.insert(h, (PathBuf::from(mp3), h0));
                        }
                    }
                }
            }
            walk(&self.root, &mut m);
            log::info!("audio: {} stream channels indexed", m.len());
            self.stream_of = Some(m);
        }
        self.stream_of.as_ref().unwrap()
    }

    fn clip_of(&mut self, file: &Path, loop_start: i32) -> Option<Arc<Clip>> {
        if let Some(c) = self.clips.get(file) {
            return c.clone();
        }
        let c = if file.extension().is_some_and(|e| e == "mp3") { decode_mp3(file) } else { crate::audio::load_wav_clip(file) };
        let c = c.map(|mut c| {
            c.loop_start = loop_start.max(0) as usize;
            Arc::new(c)
        });
        self.clips.insert(file.to_path_buf(), c.clone());
        c
    }

    // ---------------------------------------------------------------------------------------------- building

    fn make(&mut self, hash: u32, depth: usize) -> Option<Node> {
        if hash == 0 || hash == NONE || depth > 24 {
            return None;
        }
        let o = self.snd.get(hash)?.to_vec();
        let s = audcfg::parse_sound(&o);
        let h = s.header.clone();
        // variances, drawn once (sub_8213DBC0)
        let vol = (h.volume as i32 + self.rng.int(-(h.volume_var as i32), h.volume_var as i32)) as f32 * 0.01;
        let pitch = (h.pitch as i32 + self.rng.int(-(h.pitch_var as i32), h.pitch_var as i32)) as i16 as i32;
        let pan = if h.pan == -1 { -1 } else { ((h.pan as i32 + self.rng.int(-(h.pan_var as i32), h.pan_var as i32) + 360).rem_euclid(360)) as i16 };
        let predelay_ms = (h.predelay as i32 + self.rng.int(-(h.predelay_var as i32), h.predelay_var as i32)).max(0) as f32;
        let start_ms = (h.start_offset as i64 + self.rng.int(-(h.start_offset_var.min(i32::MAX as u32) as i32), h.start_offset_var.min(i32::MAX as u32) as i32) as i64).max(0) as f32;
        let kind = match s.payload {
            Payload::Simple { bank, wave } => {
                let w = self.bank_waves(bank).and_then(|m| m.get(&wave).cloned());
                if let Some(w) = w {
                    let clip = self.clip_of(&w.file.clone(), w.loop_start);
                    let dur_ms = clip.as_ref().map(|c| c.frames() as f32 * 1000.0 / c.rate as f32).unwrap_or(0.0);
                    Kind::Wave { looping: w.loop_start >= 0, wave: Some(w), clip, voice: self.voice_id(), pos_ms: 0.0, dur_ms }
                } else if let Some((mp3, h0)) = self.stream_index().get(&wave).cloned() {
                    // a stream channel: the converted file holds every channel folded to stereo
                    let clip = self.clip_of(&mp3, -1);
                    let dur_ms = clip.as_ref().map(|c| c.frames() as f32 * 1000.0 / c.rate as f32).unwrap_or(0.0);
                    Kind::Stream { clip, headroom: h0 as f32 * 0.01, voice: self.voice_id(), pos_ms: 0.0, dur_ms, looping: false }
                } else {
                    Kind::Wave { wave: None, clip: None, voice: 0, pos_ms: 0.0, dur_ms: 0.0, looping: false }
                }
            }
            Payload::Streaming { children, .. } => {
                // all channels are in one folded file: play the first channel's stream once
                let looped = h.tristate(6) == 1;
                match children.iter().find_map(|&c| self.make(c, depth + 1)) {
                    Some(mut n) => {
                        if let Kind::Stream { looping, .. } = &mut n.kind {
                            *looping = looped;
                        }
                        Kind::Group { children: vec![n] }
                    }
                    None => Kind::Empty,
                }
            }
            Payload::Wrapper { child } => Kind::Group { children: self.make(child, depth + 1).into_iter().collect() },
            Payload::Multitrack { children } => Kind::Group { children: children.iter().filter_map(|&c| self.make(c, depth + 1)).collect() },
            // approximations: the first child (TwinLoop cross-fades two instances of it; Crossfade picks by distance)
            Payload::TwinLoop { children, .. } => Kind::Group { children: children.first().and_then(|&c| self.make(c, depth + 1)).into_iter().collect() },
            Payload::Crossfade { near, .. } => Kind::Group { children: self.make(near, depth + 1).into_iter().collect() },
            Payload::VariableBlock { child, .. } | Payload::MathOperation { child, .. } => Kind::Group { children: self.make(child, depth + 1).into_iter().collect() },
            Payload::Randomized { history_len, children, .. } => {
                let pick = self.pick(hash, &h, history_len as usize, &children);
                Kind::Group { children: pick.and_then(|c| self.make(c, depth + 1)).into_iter().collect() }
            }
            Payload::Looping { count, restart_ms, child } => match self.make(child, depth + 1) {
                Some(c) => Kind::Looping { child: Box::new(c), child_hash: child, count, restart_ms, plays: 0 },
                None => Kind::Empty,
            },
            Payload::Sequential { children } => Kind::Sequential { list: children, at: 0, cur: None },
            Payload::Retriggered { count, delay_ms, child } => Kind::Retrig { child_hash: child, count, delay_ms: delay_ms as f32, next_ms: 0.0, made: 0, live: Vec::new() },
            Payload::Envelope { attack, decay, sustain, hold, release, curves, child } => match self.make(child, depth + 1) {
                Some(c) => Kind::Envelope { a: attack as f32, d: decay as f32, s: sustain as f32 * 0.01, hold: hold as f32, r: release as f32, curves, child: Box::new(c), rel_from: None },
                None => Kind::Empty,
            },
            Payload::OnStop { child, on_stop, .. } => match self.make(child, depth + 1) {
                Some(c) => Kind::OnStop { child: Box::new(c), on_stop, stopped: None },
                None => Kind::Empty,
            },
            Payload::Speech | Payload::Other(_) => Kind::Empty,
        };
        Some(Node { h, vol, pitch, pan, predelay_ms, start_ms, st: St::Waiting, t: 0.0, play_t: 0.0, release: None, kind })
    }

    fn voice_id(&mut self) -> u64 {
        self.next_voice += 1;
        VOICE_BASE + self.next_voice
    }

    /// Randomized (sub_82152F18 / sub_82152CB8): step mode with tristate 5, else a weighted pick avoiding the history.
    fn pick(&mut self, hash: u32, h: &SoundHeader, hlen: usize, children: &[(u32, f32)]) -> Option<u32> {
        let n = children.len();
        if n == 0 {
            return None;
        }
        let (idx, hist) = self.history.entry(hash).or_insert_with(|| (0, vec![0xFF; hlen])).clone();
        if h.tristate(5) == 1 {
            let i = (idx + 1) % n;
            self.history.insert(hash, (i, hist));
            return Some(children[i].0);
        }
        let excluded = |i: usize| hlen < n && hist.contains(&(i as u8));
        let total: f32 = children.iter().enumerate().filter(|(i, _)| !excluded(*i)).map(|(_, c)| c.1).sum();
        let mut r = self.rng.float(0.0, total);
        let mut chosen = 0;
        for (i, c) in children.iter().enumerate() {
            if excluded(i) {
                continue;
            }
            r -= c.1;
            if r <= 0.0 {
                chosen = i;
                break;
            }
        }
        if hlen > 0 {
            let mut hist = hist;
            let at = idx % hlen;
            hist[at] = chosen as u8;
            self.history.insert(hash, ((at + 1) % hlen, hist));
        }
        Some(children[chosen].0)
    }

    // ---------------------------------------------------------------------------------------------- game API

    /// Start a sound by name. Returns `None` when sounds.dat has no such sound.
    pub fn play(&mut self, name: &str, req: Req) -> Option<Handle> {
        self.play_hash(joaat(name), req)
    }

    pub fn play_hash(&mut self, hash: u32, req: Req) -> Option<Handle> {
        let root = self.make(hash, 0)?;
        let id = self.next;
        self.next += 1;
        self.insts.push(Instance { id, root, req, prev_dist: None, doppler: 1.0, stop: false });
        Some(Handle(id))
    }

    /// Change what the game asks of a playing sound (position, volume, pitch).
    pub fn set(&mut self, h: Handle, req: Req) {
        if let Some(i) = self.insts.iter_mut().find(|i| i.id == h.0) {
            i.req = req;
        }
    }

    /// Stop request: release times apply (OnStop sounds start their stop sound).
    pub fn stop(&mut self, h: Handle) {
        if let Some(i) = self.insts.iter_mut().find(|i| i.id == h.0) {
            i.stop = true;
        }
    }

    pub fn playing(&self, h: Handle) -> bool {
        self.insts.iter().any(|i| i.id == h.0)
    }

    pub fn count(&self) -> usize {
        self.insts.len()
    }

    /// Advance every sound by `dt` and drive the mixer's voices.
    pub fn update(&mut self, dt: f32, audio: &mut Audio) {
        let ms = dt * 1000.0;
        let mut insts = std::mem::take(&mut self.insts);
        let (lp, lr, lf) = self.listener;
        for inst in insts.iter_mut() {
            // Doppler (sub_8215F568): radial speed from the distance change, ratio c / (c + v), smoothed 50/50
            if let Some(p) = inst.req.pos {
                let d = (p - lp).length();
                if let Some(pd) = inst.prev_dist {
                    if dt > 0.0 {
                        let v = ((d - pd) * inst.req.doppler / dt).clamp(-100.0, 100.0);
                        inst.doppler = 0.5 * inst.doppler + 0.5 * (340.0 / (340.0 + v));
                    }
                }
                inst.prev_dist = Some(d);
            }
            if inst.stop {
                inst.stop = false;
                stop_node(&mut inst.root);
            }
            let ctx = Ctx { vol: inst.req.volume, pitch: inst.req.pitch, pan: inst.req.pan, category: joaat("BASE"), curve: 0, curve_scale: 1.0, doppler: 1.0, mask: 0, positional: 2 };
            let mut out = Vec::new();
            self.step(&mut inst.root, ms, ctx, 0.0, &mut out);
            for v in out {
                self.emit(v, inst, (lp, lr, lf), audio);
            }
        }
        insts.retain(|i| i.root.st != St::Done);
        // sounds started while updating (none today) stay
        insts.append(&mut self.insts);
        self.insts = insts;
    }

    /// One node: timing, its type's rule, the settings handed down; playing waves are collected in `out`.
    fn step(&mut self, n: &mut Node, ms: f32, parent: Ctx, parent_offset: f32, out: &mut Vec<Leaf>) {
        if n.st == St::Done {
            return;
        }
        n.t += ms;
        if n.st == St::Waiting {
            // predelay and start offset (sub_8213CBD8): the parent's elapsed offset first eats the predelay
            let (pd, so) = if parent_offset >= n.predelay_ms { (0.0, parent_offset - n.predelay_ms + n.start_ms) } else { (n.predelay_ms - parent_offset, n.start_ms) };
            if n.t < pd {
                return;
            }
            n.st = St::Playing;
            n.play_t = so;
        } else {
            n.play_t += ms;
        }
        // settings (sub_8213BC50 / sub_8213B0F0)
        let h = &n.h;
        let mut vol = n.vol + if h.no_inherit & 0x80 != 0 { 0.0 } else { parent.vol };
        let pitch = n.pitch + if h.no_inherit & 0x40 != 0 { 0 } else { parent.pitch };
        let ppan = if h.no_inherit & 0x20 != 0 { -1 } else { parent.pan };
        let pan = match (ppan, n.pan) {
            (-1, b) => b,
            (a, -1) => a,
            (a, b) => ((a as i32 + b as i32).rem_euclid(360)) as i16,
        };
        // attack (sub_8213C4C8) and release (sub_8213C3D0): linear amplitude, in dB
        if h.attack > 0 && n.play_t < h.attack as f32 {
            vol += lin2db((n.play_t / h.attack as f32).clamp(0.0, 1.0));
        }
        if let Some((a, b)) = n.release {
            if n.t >= b {
                n.st = St::Done;
                return;
            }
            if b.is_finite() {
                vol += lin2db(((b - n.t) / (b - a)).clamp(0.0, 1.0));
            }
        }
        let ctx = Ctx {
            vol,
            pitch,
            pan,
            category: if h.has(13) { h.category } else { parent.category },
            curve: if h.has(14) { h.curve } else { parent.curve },
            curve_scale: parent.curve_scale * if h.no_inherit2 & 0x40 != 0 { 1.0 } else { 1.0 } * h.curve_scale as f32 * 0.01,
            doppler: if h.no_inherit & 0x01 != 0 { 1.0 } else { parent.doppler } * h.doppler as f32 * 0.01,
            mask: h.speaker_mask | if h.no_inherit2 & 0x20 != 0 { 0 } else { parent.mask },
            positional: match h.tristate(1) {
                2 => parent.positional,
                v => v,
            },
        };
        let offset = n.play_t;
        let releasing = n.release.is_some();
        let done = match &mut n.kind {
            Kind::Wave { clip, wave, voice, pos_ms, dur_ms, looping } => {
                let Some(c) = clip.clone() else { n.st = St::Done; return };
                if *pos_ms == 0.0 {
                    *pos_ms = if *looping { offset % dur_ms.max(1.0) } else { offset };
                }
                if !*looping && *pos_ms >= *dur_ms {
                    true
                } else {
                    let ratio = 2f32.powf(ctx.pitch as f32 / 1200.0);
                    let headroom = wave.as_ref().map(|w| w.headroom as f32 * 0.01).unwrap_or(0.0);
                    out.push(Leaf { voice: *voice, clip: c.clone(), ctx, headroom, start_frame: (*pos_ms * 0.001 * c.rate as f32) as f64, looping: *looping, stereo: false });
                    *pos_ms += ms * ratio;
                    if *looping && *pos_ms >= *dur_ms {
                        let ls = c.loop_start as f32 * 1000.0 / c.rate as f32;
                        *pos_ms = ls + (*pos_ms - *dur_ms) % (*dur_ms - ls).max(1.0);
                    }
                    false
                }
            }
            Kind::Stream { clip, headroom, voice, pos_ms, dur_ms, looping } => {
                let Some(c) = clip.clone() else { n.st = St::Done; return };
                if !*looping && *pos_ms >= *dur_ms {
                    true
                } else {
                    let ratio = 2f32.powf(ctx.pitch as f32 / 1200.0);
                    out.push(Leaf { voice: *voice, clip: c.clone(), ctx, headroom: *headroom, start_frame: (*pos_ms * 0.001 * c.rate as f32) as f64, looping: *looping, stereo: true });
                    *pos_ms += ms * ratio;
                    if *looping && *pos_ms >= *dur_ms {
                        *pos_ms %= dur_ms.max(1.0);
                    }
                    false
                }
            }
            Kind::Group { children } => {
                for c in children.iter_mut() {
                    self.step(c, ms, ctx, offset, out);
                }
                // Multitrack with tristate 5: one finished child stops the others
                if n.h.tristate(5) == 1 && n.h.kind == 13 && children.iter().any(|c| c.st == St::Done) {
                    children.iter_mut().for_each(stop_node);
                }
                children.iter().all(|c| c.st == St::Done)
            }
            Kind::Looping { child, child_hash, count, restart_ms, plays } => {
                self.step(child, ms, ctx, offset, out);
                if child.st == St::Done {
                    *plays += 1;
                    if *count != -1 && *plays >= *count as i32 {
                        true
                    } else {
                        // made again from its reference and played from the restart offset (sub_8215DE68)
                        let restart = *restart_ms as f32;
                        match self.make(*child_hash, 1) {
                            Some(c) => **child = c,
                            None => reset_node(child),
                        }
                        child.start_ms = restart;
                        false
                    }
                } else {
                    false
                }
            }
            Kind::Sequential { list, at, cur } => {
                loop {
                    if cur.is_none() {
                        if *at >= list.len() {
                            break;
                        }
                        let hsh = list[*at];
                        *at += 1;
                        *cur = self.make(hsh, 1).map(Box::new);
                        continue;
                    }
                    let c = cur.as_mut().unwrap();
                    self.step(c, ms, ctx, 0.0, out);
                    if c.st == St::Done {
                        *cur = None;
                        continue;
                    }
                    break;
                }
                cur.is_none() && *at >= list.len()
            }
            Kind::Retrig { child_hash, count, delay_ms, next_ms, made, live } => {
                while offset >= *next_ms && (*count == -1 || *made < *count as i32) {
                    if let Some(c) = self.make(*child_hash, 1) {
                        live.push(c);
                    }
                    *made += 1;
                    *next_ms += delay_ms.max(1.0);
                }
                for c in live.iter_mut() {
                    self.step(c, ms, ctx, 0.0, out);
                }
                live.retain(|c| c.st != St::Done);
                live.is_empty() && *count != -1 && *made >= *count as i32
            }
            Kind::Envelope { a, d, s, hold, r, curves, child, rel_from } => {
                // ADSR on the child's volume (sub_8215C5B8): value = a * d * r, linear
                let t = offset;
                let cv = |me: &SoundLib, k: usize, x: f32, default: f32| me.curves.get(&curves[k]).map(|c| c.eval(x)).unwrap_or(default);
                let att = if *a > 0.0 { cv(self, 0, (t / *a).clamp(0.0, 1.0), (t / *a).clamp(0.0, 1.0)) } else { 1.0 };
                let dec = if t <= *a {
                    1.0
                } else if *d > 0.0 && t < *a + *d {
                    let x = ((t - *a) / *d).clamp(0.0, 1.0);
                    *s + (1.0 - *s) * cv(self, 1, x, 1.0 - x)
                } else {
                    *s
                };
                if rel_from.is_none() && (releasing || (*hold >= 0.0 && t >= *a + *d + *hold)) {
                    *rel_from = Some(t);
                }
                let rel = match rel_from {
                    Some(t0) if *r > 0.0 => {
                        let x = ((t - *t0) / *r).clamp(0.0, 1.0);
                        cv(self, 2, x, 1.0 - x)
                    }
                    Some(_) => 0.0,
                    None => 1.0,
                };
                let mut c2 = ctx;
                c2.vol += lin2db(att * dec * rel);
                self.step(child, ms, c2, offset, out);
                child.st == St::Done || (rel_from.is_some() && rel <= 0.0)
            }
            Kind::OnStop { child, on_stop, stopped } => {
                if n.release.is_some() && stopped.is_none() {
                    *stopped = self.make(*on_stop, 1).map(Box::new);
                }
                self.step(child, ms, ctx, offset, out);
                if let Some(s) = stopped {
                    self.step(s, ms, ctx, 0.0, out);
                    s.st == St::Done
                } else {
                    child.st == St::Done
                }
            }
            Kind::Empty => true,
        };
        if done {
            n.st = St::Done;
        }
    }

    /// A playing wave to the mixer: the voice's gain chain (sub_8214BBD0), the pan law (sub_821653A0 / sub_821651B8),
    /// pitch and Doppler.
    fn emit(&mut self, v: Leaf, inst: &Instance, listener: (Vec3, Vec3, Vec3), audio: &mut Audio) {
        let cat = self.cats.get(&v.ctx.category).copied().unwrap_or(Category { volume: 0.0, pitch: 0, lpf: 23900, hpf: 0, rolloff_scale: 1.0 });
        let (lp, lr, lf) = listener;
        let k = std::f32::consts::FRAC_1_SQRT_2;
        let mut rolloff = 0.0;
        // positional: unset counts as positional unless a pan or speaker mask is set (0x8214BCD8)
        let positional = inst.req.pos.is_some() && if v.ctx.pan == -1 && v.ctx.mask == 0 { v.ctx.positional != 0 } else { v.ctx.positional == 1 };
        let dir: Option<Vec3> = if positional {
            let p = inst.req.pos.unwrap() - lp;
            let d = p.length();
            let s = cat.rolloff_scale * v.ctx.curve_scale;
            if let Some(dd) = audcfg::scaled_distance(d, s) {
                rolloff = match self.curves.get(&v.ctx.curve) {
                    Some(c) => c.eval(dd),
                    None => audcfg::default_rolloff_db(dd),
                };
            }
            Some(Vec3::new(p.dot(lr), p.dot(lr.cross(lf)), p.dot(lf)))
        } else if v.ctx.pan != -1 {
            let a = (v.ctx.pan as f32).to_radians();
            Some(Vec3::new(a.sin(), 0.0, a.cos()))
        } else {
            None
        };
        let (gl, gr) = if v.stereo {
            (1.0, 1.0)
        } else if v.ctx.mask != 0 && !positional {
            // speaker mask in stereo (sub_821651B8): front to its side, centre / surround add 0.7071, / sqrt(bits)
            let m = v.ctx.mask;
            let mut l = if m & 1 != 0 { 1.0 } else { 0.0 };
            let mut r = if m & 2 != 0 { 1.0 } else { 0.0 };
            if m & 4 != 0 {
                l += k;
                r += k;
            }
            if m & 16 != 0 {
                l += k;
            }
            if m & 32 != 0 {
                r += k;
            }
            let bits = (m & 0x3F).count_ones().max(1) as f32;
            (l / bits.sqrt(), r / bits.sqrt())
        } else if let Some(d) = dir {
            pan_gains(d)
        } else {
            (k, k)
        };
        // A = chain - headroom, B = category + rolloff, the request's volume is in the chain's top
        let a = v.ctx.vol - v.headroom;
        let b = cat.volume + rolloff;
        let amp = db2lin(a) * db2lin(b) * self.out_gain;
        let ratio = 2f32.powf((v.ctx.pitch + cat.pitch) as f32 / 1200.0) * if positional { inst.doppler } else { 1.0 };
        audio.drive(v.voice | if inst.req.player { SLOWED } else { 0 }, &v.clip, (amp * gl, amp * gr), ratio, v.looping, v.start_frame);
    }
}

/// The stereo pan of a direction in listener space (x right, y up, z forward; sub_821653A0, folded to stereo:
/// L = FL + 0.7071 SL, R = FR + 0.7071 SR).
pub fn pan_gains(d: Vec3) -> (f32, f32) {
    let k = std::f32::consts::FRAC_1_SQRT_2;
    let flat = Vec3::new(d.x, 0.0, d.z);
    let v = if flat.length_squared() > 1e-12 { flat.normalize() } else { Vec3::X };
    let w = if d.length_squared() > 1e-12 { d.normalize() } else { Vec3::X };
    let h = v.dot(w);
    let (x, z) = (v.x, v.z);
    let e = (1.0 - h) / 2.0;
    let fl = (k * (z - x)).max(0.0) * h + e;
    let fr = (k * (x + z)).max(0.0) * h + e;
    let sl = (-k * (x + z)).max(0.0) * h + e;
    let sr = (k * (x - z)).max(0.0) * h + e;
    (fl + k * sl, fr + k * sr)
}

struct Leaf {
    voice: u64,
    clip: Arc<Clip>,
    ctx: Ctx,
    headroom: f32,
    start_frame: f64,
    looping: bool,
    stereo: bool,
}

/// A stop request down the tree (sub_8213D9D8): the release time, or done at once without one.
fn stop_node(n: &mut Node) {
    if n.st == St::Done || n.release.is_some() {
        return;
    }
    if n.h.release >= 0 && n.st == St::Playing {
        n.release = Some((n.t, n.t + n.h.release as f32));
    } else if !matches!(n.kind, Kind::OnStop { .. } | Kind::Envelope { .. }) {
        n.st = St::Done;
        return;
    } else {
        // no release time: OnStop plays its stop sound, Envelope its release stage; they end by themselves
        n.release = Some((n.t, f32::INFINITY));
    }
    match &mut n.kind {
        Kind::Group { children } => children.iter_mut().for_each(stop_node),
        // Envelope: its own release stage fades the child (the step above starts it)
        Kind::Looping { child, .. } => stop_node(child),
        Kind::Sequential { cur, .. } => {
            if let Some(c) = cur {
                stop_node(c)
            }
        }
        Kind::Retrig { live, count, made, .. } => {
            *count = *made as i16;
            live.iter_mut().for_each(stop_node)
        }
        Kind::OnStop { child, .. } => stop_node(child),
        _ => {}
    }
}

/// A node played again from the start (Looping's restart).
fn reset_node(n: &mut Node) {
    n.st = St::Waiting;
    n.t = 0.0;
    n.play_t = 0.0;
    n.release = None;
    match &mut n.kind {
        Kind::Wave { pos_ms, .. } | Kind::Stream { pos_ms, .. } => *pos_ms = 0.0,
        Kind::Group { children } => children.iter_mut().for_each(reset_node),
        Kind::Looping { child, plays, .. } => {
            *plays = 0;
            reset_node(child)
        }
        Kind::Envelope { child, rel_from, .. } => {
            *rel_from = None;
            reset_node(child)
        }
        Kind::Sequential { at, cur, .. } => {
            *at = 0;
            *cur = None;
        }
        Kind::Retrig { next_ms, made, live, .. } => {
            *next_ms = 0.0;
            *made = 0;
            live.clear();
        }
        Kind::OnStop { child, stopped, .. } => {
            *stopped = None;
            reset_node(child)
        }
        Kind::Empty => {}
    }
}

/// A converted stream (stereo MP3) decoded whole.
fn decode_mp3(p: &Path) -> Option<Clip> {
    use rodio::Source;
    let f = std::fs::File::open(p).ok()?;
    let dec = rodio::Decoder::new(std::io::BufReader::new(f)).ok()?;
    let channels = dec.channels() as usize;
    let rate = dec.sample_rate();
    let data: Vec<f32> = dec.map(|s| s as f32 / 32768.0).collect();
    if channels == 2 {
        Some(Clip { data, rate, channels: 2, loop_start: 0 })
    } else {
        // mono / other: keep the first channel
        Some(Clip::mono(data.chunks(channels.max(1)).map(|c| c[0]).collect(), rate))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pan_law() {
        let k = std::f32::consts::FRAC_1_SQRT_2;
        // straight ahead: 0.7071 each side
        let (l, r) = pan_gains(Vec3::Z);
        assert!((l - k).abs() < 1e-6 && (r - k).abs() < 1e-6);
        // right: FR = k, SR = k -> R = k + k*k; L = 0
        let (l, r) = pan_gains(Vec3::X);
        assert!(l.abs() < 1e-6 && (r - (k + 0.5)).abs() < 1e-6, "{l} {r}");
        // behind: SL = SR = k -> 0.5 each
        let (l, r) = pan_gains(Vec3::NEG_Z);
        assert!((l - 0.5).abs() < 1e-6 && (r - 0.5).abs() < 1e-6);
        // straight up: h = 0 -> every speaker 0.5
        let (l, r) = pan_gains(Vec3::Y);
        assert!((l - (0.5 + 0.5 * k)).abs() < 1e-6 && (r - l).abs() < 1e-6);
    }

    #[test]
    fn mwc_ranges() {
        let mut g = Mwc { lo: 1, carry: 0 };
        for _ in 0..1000 {
            let i = g.int(-3, 3);
            assert!((-3..=3).contains(&i));
            let f = g.float(2.0, 5.0);
            assert!((2.0..5.0).contains(&f));
        }
    }
}
