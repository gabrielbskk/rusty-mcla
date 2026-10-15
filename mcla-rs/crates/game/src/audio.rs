//! Game audio from the original sound banks.
//!
//! `tools/convert_audio.py` unwraps the disc's XMA banks into `<data>/../audio_pc`:
//! one folder per bank with a WAV per wave (named after the wave) and one MP3 per
//! stream (music, ambience). This module mixes them: engines are cross-faded between
//! the per-rpm loops of the car's bank exactly like the game's `ENGINE_XFADER` objects,
//! everything else is one-shots and looping voices with distance attenuation and pan.

use glam::Vec3;
use hashbrown::HashMap;
use rodio::{OutputStream, OutputStreamHandle, Sink, Source};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const RATE: u32 = 44100;
const TAB: char = '\u{9}';

/// Decoded mono PCM.
pub struct Clip {
    pub data: Vec<f32>,
    pub rate: u32,
}

fn load_wav(p: &Path) -> Option<Clip> {
    let d = std::fs::read(p).ok()?;
    if d.len() < 44 || &d[0..4] != b"RIFF" {
        return None;
    }
    let mut o = 12;
    let mut rate = RATE;
    let mut ch = 1usize;
    while o + 8 <= d.len() {
        let id = &d[o..o + 4];
        let n = u32::from_le_bytes(d[o + 4..o + 8].try_into().ok()?) as usize;
        let body = o + 8;
        if id == b"fmt " && body + 16 <= d.len() {
            ch = u16::from_le_bytes([d[body + 2], d[body + 3]]).max(1) as usize;
            rate = u32::from_le_bytes(d[body + 4..body + 8].try_into().ok()?);
        } else if id == b"data" {
            let end = (body + n).min(d.len());
            let data: Vec<f32> = d[body..end].chunks_exact(2 * ch).map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0).collect();
            return Some(Clip { data, rate });
        }
        o = body + n + (n & 1);
    }
    None
}

struct Voice {
    /// 0 = one-shot, otherwise the id of a looping voice the game keeps updating
    id: u64,
    clip: Arc<Clip>,
    pos: f64,
    speed: f32,
    /// current and target gain per ear (smoothed to avoid clicks)
    gl: f32,
    gr: f32,
    tl: f32,
    tr: f32,
    looping: bool,
    /// loop voices fade out and die when the game stops updating them
    fresh: bool,
}

#[derive(Default)]
struct Mixer {
    voices: Vec<Voice>,
    master: f32,
    /// (step 23b) the dynamic mixer's slow-down state: (gain, pitch) of the player's vehicle
    /// and the ambience's gain, ramped linearly from `from` to `to` over `dur` seconds
    slow: Option<SlowRamp>,
}

/// AudioManager_SetActiveTimeSlow's mixer state (DYNAMIC_MIXER_SLOWDOWN): the categories'
/// settings as the category controllers ramp them (sub_821EC620 / sub_821EC060 -> 0x82547908).
#[derive(Clone, Copy, Debug)]
pub struct SlowRamp {
    /// [player vehicle gain, player vehicle pitch, ambience gain]
    pub from: [f32; 3],
    pub to: [f32; 3],
    pub start: std::time::Instant,
    pub dur: f32,
}

impl SlowRamp {
    pub fn now(&self) -> [f32; 3] {
        let k = if self.dur > 0.0 { (self.start.elapsed().as_secs_f32() / self.dur).clamp(0.0, 1.0) } else { 1.0 };
        [0, 1, 2].map(|i| self.from[i] + (self.to[i] - self.from[i]) * k)
    }
}

/// DYNAMIC_MIXER_SLOWDOWN's settings: PLAYER_VEHICLES -1000 mB (10^(-10/20)) at pitch 0.5,
/// AMBIENCE -10000 mB (silent - sub_821493F8 gives 0 at -100 dB and under).
pub const SLOWDOWN_ON: [f32; 3] = [0.316_227_77, 0.5, 0.0];
pub const SLOWDOWN_OFF: [f32; 3] = [1.0, 1.0, 1.0];
/// the state's fade in (+30) and out (+34), seconds
pub const SLOWDOWN_FADE: (f32, f32) = (1.0, 1.0);

struct MixSource {
    mixer: Arc<Mutex<Mixer>>,
    buf: Vec<f32>,
    at: usize,
}

const BLOCK: usize = 256;

impl MixSource {
    fn refill(&mut self) {
        self.buf.clear();
        self.buf.resize(BLOCK * 2, 0.0);
        self.at = 0;
        let Ok(mut m) = self.mixer.lock() else { return };
        let master = m.master;
        // (step 23b: the slow-down state on the player's vehicle - the engine's slot 0)
        let slow = m.slow.map(|r| r.now()).unwrap_or(SLOWDOWN_OFF);
        for v in m.voices.iter_mut() {
            let mine = v.id >= 0x1000_0000 && v.id < 0x1000_0040;
            let (cat_gain, cat_pitch) = if mine { (slow[0], slow[1]) } else { (1.0, 1.0) };
            let n = v.clip.data.len();
            if n < 2 {
                v.tl = 0.0;
                v.tr = 0.0;
                v.looping = false;
                v.pos = n as f64;
                continue;
            }
            let step = (v.speed * cat_pitch * v.clip.rate as f32 / RATE as f32) as f64;
            for i in 0..BLOCK {
                v.gl += (v.tl - v.gl) * 0.004;
                v.gr += (v.tr - v.gr) * 0.004;
                let mut p = v.pos;
                if p >= (n - 1) as f64 {
                    if v.looping {
                        p %= (n - 1) as f64;
                        v.pos = p;
                    } else {
                        break;
                    }
                }
                let i0 = p as usize;
                let f = (p - i0 as f64) as f32;
                let s = v.clip.data[i0] * (1.0 - f) + v.clip.data[i0 + 1] * f;
                self.buf[i * 2] += s * v.gl * master * cat_gain;
                self.buf[i * 2 + 1] += s * v.gr * master * cat_gain;
                v.pos += step;
            }
        }
        // drop finished one-shots and faded-out loops
        m.voices.retain(|v| {
            let n = v.clip.data.len();
            let ended = !v.looping && v.pos >= n.saturating_sub(1) as f64;
            let silent = v.id != 0 && v.tl == 0.0 && v.tr == 0.0 && v.gl.abs() < 1e-4 && v.gr.abs() < 1e-4;
            !(ended || silent)
        });
        // soft clip
        for s in self.buf.iter_mut() {
            *s = (*s).tanh();
        }
    }
}

impl Iterator for MixSource {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if self.at >= self.buf.len() {
            self.refill();
        }
        let s = self.buf[self.at];
        self.at += 1;
        Some(s)
    }
}

impl Source for MixSource {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> u16 {
        2
    }
    fn sample_rate(&self) -> u32 {
        RATE
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

/// The rpm loops of one car's engine bank.
struct EngineBank {
    /// (rpm, clip), ascending
    loops: Vec<(f32, Arc<Clip>)>,
}

pub struct Audio {
    _stream: Option<OutputStream>,
    handle: Option<OutputStreamHandle>,
    _sink: Option<Sink>,
    /// offline rendering (tests): the mix is pulled by the game loop and kept here
    offline: Option<(MixSource, Vec<f32>)>,
    mixer: Arc<Mutex<Mixer>>,
    root: PathBuf,
    clips: HashMap<String, Option<Arc<Clip>>>,
    engines: HashMap<String, Option<Arc<EngineBank>>>,
    /// wave names per bank folder (from `_index.tsv`)
    index: HashMap<String, Arc<Vec<String>>>,
    music: Option<Sink>,
    /// one-shot dialogue stream (story scenes)
    voice: Option<Sink>,
    music_files: Vec<PathBuf>,
    music_next: usize,
    pub music_on: bool,
    pub music_volume: f32,
    pub now_playing: String,
    /// per music file: the song's title and genre (0 eclectic .. 6 west coast rap) from the
    /// game's string table; None for what is not one of its songs
    music_meta: Vec<Option<(String, u8)>>,
    /// the playlist: one genre, or all of them
    pub playlist: Option<u8>,
    /// the original's music lists (`mc4`): with them the next song is theirs
    pub lists: Option<crate::mc4::MusicShared>,
    /// set when a new track starts (the HUD shows the title once)
    pub track_changed: bool,
    /// looping stereo bed (city ambience): (path, sink)
    bed: Option<(PathBuf, Sink)>,
    /// listener
    ear: Vec3,
    ear_right: Vec3,
    rng: u32,
    pub available: bool,
}

/// Gains for a sound at `pos`: inverse-distance roll-off with a reference distance,
/// constant-power pan from the listener's right vector.
fn spatial(ear: Vec3, right: Vec3, pos: Vec3, vol: f32, ref_dist: f32) -> (f32, f32) {
    let d = pos - ear;
    let dist = d.length();
    let att = (ref_dist / dist.max(ref_dist)).powf(1.3);
    let pan = if dist > 0.5 { (d / dist).dot(right).clamp(-1.0, 1.0) * 0.8 } else { 0.0 };
    let a = (pan + 1.0) * std::f32::consts::FRAC_PI_4;
    (vol * att * a.cos() * 1.414, vol * att * a.sin() * 1.414)
}

impl Audio {
    pub fn new(data_dir: &str) -> Option<Audio> {
        Self::create(data_dir, false)
    }

    /// No audio device: the game loop pulls the mix with [`Audio::render`] and
    /// [`Audio::write_capture`] saves it.
    pub fn new_offline(data_dir: &str) -> Option<Audio> {
        Self::create(data_dir, true)
    }

    fn create(data_dir: &str, offline: bool) -> Option<Audio> {
        let mixer = Arc::new(Mutex::new(Mixer { voices: Vec::new(), master: 0.9, slow: None }));
        let src = MixSource { mixer: mixer.clone(), buf: Vec::new(), at: 0 };
        let (stream, handle, sink, off) = if offline {
            (None, None, None, Some((src, Vec::new())))
        } else {
            let (stream, handle) = OutputStream::try_default().ok()?;
            let sink = Sink::try_new(&handle).ok()?;
            sink.append(src);
            sink.play();
            (Some(stream), Some(handle), Some(sink), None)
        };
        // <data>/../audio_pc  (data = .../extracted/cache)
        let root = Path::new(data_dir).parent().map(|p| p.join("audio_pc")).unwrap_or_else(|| PathBuf::from("audio_pc"));
        let available = root.is_dir();
        if !available {
            log::warn!("audio: {} not found - run tools/convert_audio.py to convert the original sound banks", root.display());
        }
        // the radio: full songs only (short files are stingers and menu loops)
        let mut music_files: Vec<PathBuf> = std::fs::read_dir(root.join("music"))
            .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.extension().map_or(false, |x| x == "mp3") && p.metadata().map_or(false, |m| m.len() > 1_500_000)).collect())
            .unwrap_or_default();
        music_files.sort();
        log::info!("audio: bank root {} ({}), {} music tracks", root.display(), if available { "ok" } else { "missing" }, music_files.len());
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(7) | 1;
        Some(Audio {
            _stream: stream,
            handle,
            _sink: sink,
            offline: off,
            mixer,
            root,
            clips: HashMap::new(),
            engines: HashMap::new(),
            index: HashMap::new(),
            music: None,
            music_next: (seed as usize) % music_files.len().max(1),
            music_files,
            music_on: true,
            music_volume: 0.35,
            now_playing: String::new(),
            music_meta: Vec::new(),
            playlist: None,
            lists: None,
            track_changed: false,
            bed: None,
            voice: None,
            ear: Vec3::ZERO,
            ear_right: Vec3::X,
            rng: seed,
            available,
        })
    }

    /// Offline mode: advance the mix by `dt` seconds.
    pub fn render(&mut self, dt: f32) {
        if let Some((src, out)) = self.offline.as_mut() {
            let n = (dt * RATE as f32).round() as usize * 2;
            for _ in 0..n {
                out.push(src.next().unwrap_or(0.0));
            }
        }
    }

    /// Offline mode: write everything rendered so far as a 16-bit stereo WAV.
    pub fn write_capture(&self, path: &str) {
        let Some((_, out)) = self.offline.as_ref() else { return };
        let mut d = Vec::with_capacity(44 + out.len() * 2);
        let n = (out.len() * 2) as u32;
        d.extend_from_slice(b"RIFF");
        d.extend_from_slice(&(36 + n).to_le_bytes());
        d.extend_from_slice(b"WAVEfmt ");
        d.extend_from_slice(&16u32.to_le_bytes());
        d.extend_from_slice(&1u16.to_le_bytes());
        d.extend_from_slice(&2u16.to_le_bytes());
        d.extend_from_slice(&RATE.to_le_bytes());
        d.extend_from_slice(&(RATE * 4).to_le_bytes());
        d.extend_from_slice(&4u16.to_le_bytes());
        d.extend_from_slice(&16u16.to_le_bytes());
        d.extend_from_slice(b"data");
        d.extend_from_slice(&n.to_le_bytes());
        for s in out {
            d.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
        }
        let _ = std::fs::write(path, d);
        log::info!("audio: wrote {} ({:.1} s)", path, out.len() as f32 / (RATE as f32 * 2.0));
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn random(&mut self) -> u32 {
        self.rand()
    }

    fn rand(&mut self) -> u32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        x
    }

    pub fn set_listener(&mut self, pos: Vec3, right: Vec3) {
        self.ear = pos;
        self.ear_right = right.normalize_or(Vec3::X);
    }

    /// (step 23b) AudioManager_SetActiveTimeSlow(on) (sub_821E7470 -> sub_821E68D0): the dynamic
    /// mixer's state 16 switched on - its enter sound, the categories ramped in over 1 s - or
    /// off (no exit sound: +18 is NULL_SOUND), ramped back over 1 s. Only a change acts.
    pub fn set_time_slow(&mut self, on: bool) {
        let Ok(mut m) = self.mixer.lock() else { return };
        let was = m.slow.map_or(false, |r| r.to == SLOWDOWN_ON);
        if was == on {
            return;
        }
        let from = m.slow.map(|r| r.now()).unwrap_or(SLOWDOWN_OFF);
        let (to, dur) = if on { (SLOWDOWN_ON, SLOWDOWN_FADE.0) } else { (SLOWDOWN_OFF, SLOWDOWN_FADE.1) };
        m.slow = Some(SlowRamp { from, to, start: std::time::Instant::now(), dur });
        drop(m);
        if on {
            self.play("special_effects/bnk_special_effects/stinger_01_enter_l", 0.8);
        }
        log::info!("audio: time slow {}", if on { "on (DYNAMIC_MIXER_SLOWDOWN)" } else { "off" });
    }

    /// The ambience's share under the slow-down state (AMBIENCE's category gain).
    pub fn ambience_gain(&self) -> f32 {
        self.mixer.lock().ok().and_then(|m| m.slow.map(|r| r.now()[2])).unwrap_or(1.0)
    }

    pub fn set_master(&self, v: f32) {
        if let Ok(mut m) = self.mixer.lock() {
            m.master = v;
        }
    }

    /// Number of voices currently mixing (debug).
    pub fn voices(&self) -> usize {
        self.mixer.lock().map(|m| m.voices.len()).unwrap_or(0)
    }

    /// Wave names of a bank folder.
    pub fn bank_index(&mut self, bank: &str) -> Arc<Vec<String>> {
        if let Some(i) = self.index.get(bank) {
            return i.clone();
        }
        let names: Vec<String> = std::fs::read_to_string(self.root.join(bank).join("_index.tsv")).map(|t| t.lines().filter_map(|l| l.split(TAB).next().map(|s| s.to_string())).filter(|s| !s.is_empty()).collect()).unwrap_or_default();
        let a = Arc::new(names);
        self.index.insert(bank.to_string(), a.clone());
        a
    }

    /// `bank/wave` (lower case, as written by the converter).
    pub fn clip(&mut self, key: &str) -> Option<Arc<Clip>> {
        if let Some(c) = self.clips.get(key) {
            return c.clone();
        }
        let c = load_wav(&self.root.join(format!("{key}.wav"))).map(Arc::new);
        if c.is_none() && self.available {
            log::warn!("audio: missing clip {}", key);
        }
        self.clips.insert(key.to_string(), c.clone());
        c
    }

    /// A random wave of `bank` whose name starts with `prefix`.
    pub fn pick(&mut self, bank: &str, prefix: &str) -> Option<String> {
        let idx = self.bank_index(bank);
        let m: Vec<&String> = idx.iter().filter(|n| n.starts_with(prefix)).collect();
        if m.is_empty() {
            return None;
        }
        let k = self.rand() as usize % m.len();
        Some(format!("{}/{}", bank, m[k]))
    }

    fn push(&mut self, v: Voice) {
        if let Ok(mut m) = self.mixer.lock() {
            if m.voices.len() < 96 {
                m.voices.push(v);
            }
        }
    }

    /// Non-positional one-shot (UI, the player's own effects).
    pub fn play(&mut self, key: &str, vol: f32) {
        self.play_ex(key, vol, 1.0);
    }

    pub fn play_ex(&mut self, key: &str, vol: f32, speed: f32) {
        if let Some(clip) = self.clip(key) {
            self.push(Voice { id: 0, clip, pos: 0.0, speed, gl: vol, gr: vol, tl: vol, tr: vol, looping: false, fresh: true });
        }
    }

    /// Positional one-shot.
    pub fn play_at(&mut self, key: &str, pos: Vec3, vol: f32, ref_dist: f32) {
        if let Some(clip) = self.clip(key) {
            let (l, r) = spatial(self.ear, self.ear_right, pos, vol, ref_dist);
            if l + r > 0.004 {
                self.push(Voice { id: 0, clip, pos: 0.0, speed: 1.0, gl: l, gr: r, tl: l, tr: r, looping: false, fresh: true });
            }
        }
    }

    /// Create or update a looping voice. Loops that are not updated for a frame fade out
    /// (see [`Audio::end_frame`]).
    pub fn set_loop(&mut self, id: u64, key: &str, gains: (f32, f32), speed: f32) {
        let Some(clip) = self.clip(key) else { return };
        self.set_loop_clip(id, clip, gains, speed);
    }

    fn set_loop_clip(&mut self, id: u64, clip: Arc<Clip>, gains: (f32, f32), speed: f32) {
        let Ok(mut m) = self.mixer.lock() else { return };
        if let Some(v) = m.voices.iter_mut().find(|v| v.id == id) {
            v.tl = gains.0;
            v.tr = gains.1;
            v.speed = speed;
            v.fresh = true;
            return;
        }
        if gains.0 + gains.1 < 0.002 || m.voices.len() >= 96 {
            return;
        }
        // start at a random phase so identical loops do not comb
        let start = (id.wrapping_mul(2654435761) % clip.data.len().max(1) as u64) as f64;
        m.voices.push(Voice { id, clip, pos: start, speed, gl: 0.0, gr: 0.0, tl: gains.0, tr: gains.1, looping: true, fresh: true });
    }

    pub fn gains_at(&self, pos: Vec3, vol: f32, ref_dist: f32) -> (f32, f32) {
        spatial(self.ear, self.ear_right, pos, vol, ref_dist)
    }

    /// Fade out every loop the game did not touch this frame.
    pub fn end_frame(&mut self) {
        if let Ok(mut m) = self.mixer.lock() {
            for v in m.voices.iter_mut().filter(|v| v.id != 0) {
                if !v.fresh {
                    v.tl = 0.0;
                    v.tr = 0.0;
                }
                v.fresh = false;
            }
        }
    }

    fn engine_bank(&mut self, car: &str) -> Option<Arc<EngineBank>> {
        if let Some(b) = self.engines.get(car) {
            return b.clone();
        }
        let lc = car.to_ascii_lowercase();
        // folders are named after the car; a few only survive as the hash of that name
        // cars whose own bank is not on the disc borrow the closest relative
        let relative = match lc.as_str() {
            "vp_frd_boss302_69" => "vp_frd_mustang_06",
            "vpd_am_db9_08" => "vp_am_vantage_07",
            "vpd_aud_s5_08" => "vp_aud_rs4_07",
            "vpd_cad_xlrv_08" => "vp_chv_corvettez06_07",
            "vpd_chv_chevelle_70" => "vp_chv_camaross_69",
            "vpd_duc_1098_07" => "vp_duc_999r_06",
            "vpd_lr_hse_08" | "vpd_lr_sport_08" => "vp_cry_300c_06",
            "vpd_mbz_cl65amg_08" => "vp_mbz_cl65_06",
            "vpd_mbz_clk63amg_08" => "vp_mbz_sl65_07",
            "vpd_mit_eclipse_09" => "vp_mit_eclipse_99",
            "vpd_mit_evox_08" => "vp_mit_evo_06",
            _ => "",
        };
        let as_vp = lc.replacen("vpd_", "vp_", 1);
        let cands = [
            format!("player_vehicles/0x{:08x}", mcla_formats::hash::joaat(&as_vp)),
            format!("player_vehicles/{relative}"),
            format!("player_vehicles/0x{:08x}", mcla_formats::hash::joaat(relative)),
            format!("player_vehicles/{lc}"),
            format!("player_vehicles/0x{:08x}", mcla_formats::hash::joaat(&lc)),
            format!("player_vehicles/{}", lc.replace("_dub", "")),
            // opponent_vehicles/<car>_o
            format!("0xb31871af/0x{:08x}", mcla_formats::hash::joaat(&format!("{lc}_o"))),
        ];
        let mut bank = None;
        for folder in cands {
            let idx = self.bank_index(&folder);
            let mut loops: Vec<(f32, Arc<Clip>)> = Vec::new();
            for name in idx.iter() {
                // "..._<rpm>" or "..._<rpm>_fin"
                let base = name.trim_end_matches("_fin");
                let Some(num) = base.rsplit('_').next().and_then(|t| t.parse::<f32>().ok()) else { continue };
                if !(500.0..=14000.0).contains(&num) || num % 250.0 != 0.0 {
                    continue;
                }
                if let Some(c) = self.clip(&format!("{folder}/{name}")) {
                    loops.push((num, c));
                }
            }
            if loops.len() >= 3 {
                loops.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
                log::info!("audio: engine bank {} with {} rpm loops ({}..{})", folder, loops.len(), loops[0].0, loops[loops.len() - 1].0);
                bank = Some(Arc::new(EngineBank { loops }));
                break;
            }
        }
        self.engines.insert(car.to_string(), bank.clone());
        bank
    }

    /// Drive one engine: cross-fade between the two recorded loops around `rpm`, each
    /// pitched to the exact engine speed. `slot` distinguishes simultaneous engines.
    pub fn engine(&mut self, slot: u64, car: &str, rpm: f32, throttle: f32, gains: (f32, f32)) -> bool {
        let Some(bank) = self.engine_bank(car) else { return false };
        let n = bank.loops.len();
        let lo = bank.loops[0].0;
        let hi = bank.loops[n - 1].0;
        let r = rpm.clamp(lo * 0.8, hi * 1.12);
        let mut i = 0;
        while i + 2 < n && bank.loops[i + 1].0 <= r {
            i += 1;
        }
        let (r0, r1) = (bank.loops[i].0, bank.loops[i + 1].0);
        let f = ((r - r0) / (r1 - r0).max(1.0)).clamp(0.0, 1.0);
        // off-throttle the engine is quieter
        let load = 0.55 + 0.45 * throttle.clamp(0.0, 1.0);
        for (k, (lr, clip)) in bank.loops.iter().enumerate() {
            let w = if k == i { (1.0 - f).sqrt() } else if k == i + 1 { f.sqrt() } else { 0.0 };
            let id = 0x1000_0000 + slot * 64 + k as u64;
            if w > 0.0 {
                self.set_loop_clip(id, clip.clone(), (gains.0 * w * load, gains.1 * w * load), r / lr);
            }
        }
        true
    }

    /// Looping stereo background stream (city ambience). `None` stops it.
    pub fn set_bed(&mut self, rel: Option<&str>, volume: f32) {
        let Some(handle) = self.handle.as_ref() else { return };
        let want = rel.map(|r| self.root.join(r));
        // (step 23b: AMBIENCE under the slow-down state)
        let volume = volume * self.ambience_gain();
        if let (Some((cur, sink)), Some(w)) = (self.bed.as_ref(), want.as_ref()) {
            if cur == w {
                sink.set_volume(volume);
                return;
            }
        }
        if let Some((_, s)) = self.bed.take() {
            s.stop();
        }
        let Some(p) = want else { return };
        let Ok(f) = std::fs::File::open(&p) else { return };
        let Ok(dec) = rodio::Decoder::new(std::io::BufReader::new(f)) else { return };
        if let Ok(s) = Sink::try_new(handle) {
            s.set_volume(volume);
            s.append(dec.repeat_infinite());
            s.play();
            self.bed = Some((p, s));
        }
    }

    /// Play a stereo stream once (story scene dialogue); `None` stops it.
    pub fn play_stream(&mut self, rel: Option<&str>, volume: f32) {
        if let Some(s) = self.voice.take() {
            s.stop();
        }
        let (Some(handle), Some(rel)) = (self.handle.as_ref(), rel) else { return };
        let Ok(f) = std::fs::File::open(self.root.join(rel)) else { return };
        let Ok(dec) = rodio::Decoder::new(std::io::BufReader::new(f)) else { return };
        if let Ok(s) = Sink::try_new(handle) {
            s.set_volume(volume);
            s.append(dec);
            s.play();
            self.voice = Some(s);
        }
    }

    /// Keep the radio playing: starts the next track when the current one ends.
    pub fn update_music(&mut self, playing: bool) {
        if !self.music_on || !playing || self.music_files.is_empty() {
            if let Some(s) = self.music.as_ref() {
                s.pause();
            }
            return;
        }
        if let Some(s) = self.music.as_ref() {
            s.set_volume(self.music_volume);
            if s.is_paused() {
                s.play();
            }
            if !s.empty() {
                return;
            }
        }
        self.next_track();
    }

    /// Title of a track from the converter's channel list ("ARTIST_SONG_LEFT").
    fn track_title(p: &Path) -> String {
        let txt = std::fs::read_to_string(format!("{}.txt", p.display())).unwrap_or_default();
        let name = txt.lines().next().and_then(|l| l.split(TAB).nth(2)).unwrap_or("?").to_string();
        if name == "?" {
            return String::new();
        }
        name.trim_end_matches("_LEFT").trim_end_matches("_RIGHT").trim_end_matches("_L").trim_end_matches("_R").replace('_', " ")
    }

    /// Name the songs: the string table's `Music_<Genre>_<Artist>_<Song>` keys against the
    /// asset names of the converted tracks ("NAS_HERO").
    pub fn set_music_catalog(&mut self, strings: &mcla_formats::strtbl::StringTable) {
        const GENRES: [(&str, u8); 9] = [("eclectic_", 0), ("electronic_", 1), ("hardrock_", 2), ("hard_rock_", 2), ("hiphop_", 3), ("rock_", 4), ("techno_", 5), ("westcoastrap_", 6), ("west_rap_", 6)];
        let squash = |s: &str| s.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_uppercase();
        let mut songs: HashMap<String, (String, u8)> = HashMap::new();
        let mut by_song: HashMap<String, Option<(String, u8)>> = HashMap::new();
        for k in &strings.keys {
            let low = k.to_ascii_lowercase();
            let Some(rest) = low.strip_prefix("music_") else { continue };
            let Some((g, id)) = GENRES.iter().find(|(g, _)| rest.starts_with(g)) else { continue };
            if let Some(title) = strings.get(k) {
                songs.entry(squash(&rest[g.len()..])).or_insert((title.to_string(), *id));
                // (some assets carry another artist's name: the song's own name finds them)
                let song = squash(rest.rsplit('_').next().unwrap_or(""));
                if song.len() >= 4 {
                    by_song.entry(song).and_modify(|e: &mut Option<(String, u8)>| *e = None).or_insert(Some((title.to_string(), *id)));
                }
            }
        }
        self.music_meta = self
            .music_files
            .iter()
            .map(|p| {
                let txt = std::fs::read_to_string(format!("{}.txt", p.display())).unwrap_or_default();
                let asset = txt.lines().next().and_then(|l| l.split('\t').nth(2)).unwrap_or("").to_string();
                let asset = asset.trim_end_matches("_LEFT").trim_end_matches("_RIGHT").trim_end_matches("_L").trim_end_matches("_R");
                if asset.is_empty() || asset == "?" || asset.starts_with("MUSIC_") {
                    return None;
                }
                // (a song the table does not name plays with all genres only, under its asset's name)
                songs
                    .get(&squash(asset))
                    .cloned()
                    .or_else(|| by_song.get(&squash(asset.rsplit('_').next().unwrap_or(""))).cloned().flatten())
                    .or_else(|| Some((asset.replace('_', " "), 255)))
            })
            .collect();
        log::info!("audio: {} of {} music tracks are songs of the game's playlists, {} more play with all genres", self.music_meta.iter().flatten().filter(|m| m.1 != 255).count(), self.music_files.len(), self.music_meta.iter().flatten().filter(|m| m.1 == 255).count());
    }

    /// The songs of the game's playlists: "ARTIST - TITLE" and the genre (0..6; 255 = no
    /// genre of its own), for the pause menu's song lists.
    pub fn songs(&self) -> Vec<(String, u8)> {
        self.music_meta.iter().flatten().cloned().collect()
    }

    /// The converted tracks' asset names ("NIN_DISCIPLINE"; empty for none), file by file.
    pub fn music_assets(&self) -> Vec<String> {
        self.music_files
            .iter()
            .map(|p| {
                let txt = std::fs::read_to_string(format!("{}.txt", p.display())).unwrap_or_default();
                let asset = txt.lines().next().and_then(|l| l.split('\t').nth(2)).unwrap_or("").to_string();
                asset.trim_end_matches("_LEFT").trim_end_matches("_RIGHT").trim_end_matches("_L").trim_end_matches("_R").to_string()
            })
            .collect()
    }

    /// The song before, by the music lists' order (the pad's MUSIC_DOWN).
    pub fn prev_track(&mut self) {
        if let Some(f) = self.lists.as_ref().and_then(|l| l.lock().ok().and_then(|mut l| l.prev_file())).filter(|f| *f < self.music_files.len()) {
            self.music_next = f;
            self.start_track();
        }
    }

    /// The song the music lists say is on starts (PLAY in the playlist menu).
    pub fn play_listed(&mut self) {
        if let Some(f) = self.lists.as_ref().and_then(|l| l.lock().ok().and_then(|l| l.now_file())).filter(|f| *f < self.music_files.len()) {
            self.music_next = f;
            self.start_track();
        }
    }

    pub fn next_track(&mut self) {
        if self.music_files.is_empty() || self.handle.is_none() {
            return;
        }
        // (the original's lists: the next place of the playing list's order)
        if let Some(f) = self.lists.as_ref().and_then(|l| l.lock().ok().and_then(|mut l| l.next_file())).filter(|f| *f < self.music_files.len()) {
            self.music_next = f;
            self.start_track();
            return;
        }
        let step = 1 + self.rand() as usize % 7;
        self.music_next = (self.music_next + step) % self.music_files.len();
        // (the radio plays the songs of the chosen playlist; tracks the string table does not
        // name are menu and garage music)
        if self.music_meta.iter().any(|m| m.is_some()) {
            let ok = |m: &Option<(String, u8)>, pl: Option<u8>| m.as_ref().map_or(false, |(_, g)| pl.map_or(true, |p| p == *g));
            let pl = if self.music_meta.iter().any(|m| ok(m, self.playlist)) { self.playlist } else { None };
            for _ in 0..self.music_files.len() {
                if ok(&self.music_meta[self.music_next], pl) {
                    break;
                }
                self.music_next = (self.music_next + 1) % self.music_files.len();
            }
        }
        self.start_track();
    }

    /// The track `music_next` starts.
    fn start_track(&mut self) {
        if self.music_files.is_empty() || self.handle.is_none() {
            return;
        }
        let p = self.music_files[self.music_next].clone();
        let Ok(f) = std::fs::File::open(&p) else { return };
        let Ok(dec) = rodio::Decoder::new(std::io::BufReader::new(f)) else {
            log::warn!("audio: cannot decode {}", p.display());
            return;
        };
        if let Some(old) = self.music.take() {
            old.stop();
        }
        let Some(handle) = self.handle.as_ref() else { return };
        if let Ok(s) = Sink::try_new(handle) {
            s.set_volume(self.music_volume);
            s.append(dec);
            s.play();
            self.now_playing = match self.music_meta.get(self.music_next).cloned().flatten() {
                Some((title, _)) => title,
                None => Self::track_title(&p),
            };
            // (a song of the lists: the title the string table has for it)
            if let Some(t) = self.lists.as_ref().and_then(|l| l.lock().ok().and_then(|l| l.title_of_file(self.music_next))) {
                self.now_playing = t;
            }
            self.track_changed = true;
            log::info!("audio: now playing {} ({})", self.now_playing, p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
            self.music = Some(s);
        }
    }
}
