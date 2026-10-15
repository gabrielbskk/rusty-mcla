//! Career speech: the phone calls and radio chatter of `tune/audio/CareerScript.xml`
//! (scenarios such as `SIDEKICK_<MISSION>`: a script of `PlayAsset` / `WaitTime` steps).
//! An asset's clips are named `joaat("<ASSET>_NN")` in the `career_speech` banks; the
//! player's lines live in banks of their own, the same asset name in another bank is the
//! other speaker.

use crate::audio::Audio;
use hashbrown::{HashMap, HashSet};
use mcla_formats::hash::joaat;
use std::collections::VecDeque;
use std::path::Path;

#[derive(Clone, Debug)]
enum Step {
    /// `voice`: the profile whose banks say it (GH_01 ...), None - any
    Play { asset: String, player: bool, voice: Option<String> },
    Wait(f32),
}

#[derive(Default)]
pub struct Speech {
    /// each scenario's `<Script>` blocks: alternatives, one of them is said
    scenarios: HashMap<String, Vec<Vec<Step>>>,
    /// clip hash -> clips ("career_speech/<bank>/<file>")
    clips: HashMap<u32, Vec<String>>,
    player_banks: HashSet<String>,
    /// profile (GH_01, SH_02, PLAYER_01 ..) -> its banks ("career_speech/0x<joaat(bank name)>")
    voice_banks: HashMap<String, HashSet<String>>,
    queue: VecDeque<Step>,
    wait: f32,
    rng: u32,
}

fn attr<'a>(s: &'a str, name: &str) -> Option<&'a str> {
    let k = format!("{name}=\"");
    let i = s.find(&k)? + k.len();
    let e = s[i..].find('"')?;
    Some(&s[i..i + e])
}

impl Speech {
    pub fn load(data_dir: &str, audio_root: &Path) -> Speech {
        let mut sp = Speech { rng: 0x2545_f491, ..Default::default() };
        // clip index of the speech banks
        if let Ok(rd) = std::fs::read_dir(audio_root.join("career_speech")) {
            for e in rd.flatten() {
                let bank = e.file_name().to_string_lossy().to_string();
                let Ok(t) = std::fs::read_to_string(e.path().join("_index.tsv")) else { continue };
                for line in t.lines() {
                    let mut c = line.split('\t');
                    let (Some(name), Some(hash)) = (c.next(), c.next()) else { continue };
                    if let Ok(h) = u32::from_str_radix(hash.trim(), 16) {
                        sp.clips.entry(h).or_default().push(format!("career_speech/{bank}/{name}"));
                    }
                }
            }
        }
        // the banks' names (speech.dat: CAREER_SPEECH\[SC_]GH_01_MARTIN_RADIO ...): whose voice each is
        if let Ok(d) = std::fs::read(Path::new(data_dir).join("audio/X360/config/speech.dat")) {
            let pat = b"CAREER_SPEECH\\";
            let mut i = 0;
            while let Some(k) = d[i..].windows(pat.len()).position(|w| w == pat) {
                let st = i + k + pat.len();
                let end = d[st..].iter().position(|c| !(c.is_ascii_alphanumeric() || *c == b'_')).map_or(d.len(), |e| st + e);
                let name = String::from_utf8_lossy(&d[st..end]).to_string();
                let core = name.strip_prefix("SC_").unwrap_or(&name);
                let mut parts = core.splitn(3, '_');
                if let (Some(a), Some(b)) = (parts.next(), parts.next()) {
                    sp.voice_banks.entry(format!("{a}_{b}").to_ascii_uppercase()).or_default().insert(format!("career_speech/0x{:08x}", joaat(&name)));
                }
                i = end.max(st);
            }
        }
        // the career's script, and the South Central add-on's beside it
        let read = |name: &str| std::fs::read(Path::new(data_dir).join("tune/audio").join(name)).map(|b| String::from_utf8_lossy(&b).to_string());
        let Ok(mut t) = read("CareerScript.xml") else { return sp };
        if let Ok(sc) = read("SC_CareerScript.xml") {
            t.push_str(&sc);
        }
        let mut player_only: HashMap<String, u32> = HashMap::new();
        for sc in t.split("<Scenario ").skip(1) {
            let Some(name) = attr(sc, "name") else { continue };
            let body = sc.split("</Scenario>").next().unwrap_or("");
            let mut scripts: Vec<Vec<Step>> = Vec::new();
            let mut steps = Vec::new();
            for line in body.lines() {
                let l = line.trim();
                if l.starts_with("<!--") {
                    continue;
                }
                // (step 19d: a new <Script> is another way of saying it)
                if l.starts_with("<Script") && !steps.is_empty() {
                    scripts.push(std::mem::take(&mut steps));
                }
                if l.starts_with("<PlayAsset") {
                    let Some(asset) = attr(l, "name") else { continue };
                    let player = attr(l, "VoiceGroup") == Some("PLAYER");
                    if player {
                        // banks that hold this player line (unambiguous ones tell us whose banks they are)
                        if let Some(c) = sp.clips.get(&joaat(&format!("{asset}_01"))) {
                            if c.len() == 1 {
                                let bank = c[0].rsplit_once('/').map(|x| x.0.to_string()).unwrap_or_default();
                                *player_only.entry(bank).or_default() += 1;
                            }
                        }
                    }
                    steps.push(Step::Play { asset: asset.to_string(), player, voice: None });
                } else if l.starts_with("<WaitTime") {
                    steps.push(Step::Wait(attr(l, "value").and_then(|v| v.parse().ok()).unwrap_or(0.4)));
                }
            }
            if !steps.is_empty() {
                scripts.push(steps);
            }
            if !scripts.is_empty() {
                sp.scenarios.insert(name.to_ascii_uppercase(), scripts);
            }
        }
        // the player's voice: banks that hold many player-only lines
        sp.player_banks = player_only.into_iter().filter(|(_, n)| *n >= 8).map(|(b, _)| b).collect();
        log::info!("speech: {} scenarios, {} clips, {} player banks", sp.scenarios.len(), sp.clips.len(), sp.player_banks.len());
        sp
    }

    pub fn has(&self, scenario: &str) -> bool {
        self.scenarios.contains_key(&scenario.to_ascii_uppercase())
    }

    pub fn busy(&self) -> bool {
        !self.queue.is_empty() || self.wait > 0.0
    }

    /// Queue a scenario; returns false if it does not exist.
    pub fn say(&mut self, scenario: &str) -> bool {
        let Some(scripts) = self.scenarios.get(&scenario.to_ascii_uppercase()) else { return false };
        self.rng = self.rng.wrapping_mul(1664525).wrapping_add(1013904223);
        let steps = scripts[(self.rng >> 16) as usize % scripts.len()].clone();
        if !self.queue.is_empty() {
            self.queue.push_back(Step::Wait(0.8));
        }
        self.queue.extend(steps);
        true
    }

    /// Queue a scenario said in a character's voice (`profile` = GH_01 ..: the racer's - the
    /// original passes the racer, whose character set is the voice of CareerVoices.xml); the
    /// player's lines stay the player's. False if it does not exist.
    pub fn say_as(&mut self, scenario: &str, profile: &str) -> bool {
        let Some(scripts) = self.scenarios.get(&scenario.to_ascii_uppercase()) else { return false };
        self.rng = self.rng.wrapping_mul(1664525).wrapping_add(1013904223);
        let steps = scripts[(self.rng >> 16) as usize % scripts.len()].clone();
        if !self.queue.is_empty() {
            self.queue.push_back(Step::Wait(0.8));
        }
        let voice = (!profile.is_empty()).then(|| profile.to_ascii_uppercase());
        self.queue.extend(steps.into_iter().map(|s| match s {
            Step::Play { asset, player: false, .. } => Step::Play { asset, player: false, voice: voice.clone() },
            o => o,
        }));
        true
    }

    /// The banks of a profile's voice (a test aid).
    pub fn voice_banks(&self, profile: &str) -> usize {
        self.voice_banks.get(&profile.to_ascii_uppercase()).map_or(0, |b| b.len())
    }

    pub fn stop(&mut self) {
        self.queue.clear();
    }

    fn resolve(&mut self, asset: &str, player: bool, voice: Option<&str>) -> Option<String> {
        // numbered takes of the same line
        let mut takes: Vec<&Vec<String>> = Vec::new();
        for n in 1..=8 {
            match self.clips.get(&joaat(&format!("{asset}_{n:02}"))) {
                Some(c) => takes.push(c),
                None => break,
            }
        }
        // (in the speaker's own voice: the takes his banks hold, one of them)
        if let Some(vb) = voice.and_then(|v| self.voice_banks.get(v)) {
            let bank_of = |k: &String| k.rsplit_once('/').map(|x| x.0.to_string()).unwrap_or_default();
            let mine: Vec<String> = takes.iter().filter_map(|t| t.iter().find(|k| vb.contains(&bank_of(k))).cloned()).collect();
            if !mine.is_empty() {
                self.rng = self.rng.wrapping_mul(1664525).wrapping_add(1013904223);
                let k = mine[(self.rng >> 16) as usize % mine.len()].clone();
                log::info!("speech: {} in {}'s voice: {}", asset, voice.unwrap_or(""), k);
                return Some(k);
            }
        }
        if takes.is_empty() {
            return None;
        }
        self.rng = self.rng.wrapping_mul(1664525).wrapping_add(1013904223);
        let take = takes[(self.rng >> 16) as usize % takes.len()];
        let is_player = |k: &String| k.rsplit_once('/').map_or(false, |x| self.player_banks.contains(x.0));
        take.iter().find(|k| is_player(k) == player).or(take.first()).cloned()
    }

    pub fn update(&mut self, a: &mut Audio, dt: f32) {
        self.wait -= dt;
        while self.wait <= 0.0 {
            let Some(step) = self.queue.pop_front() else {
                self.wait = 0.0;
                return;
            };
            match step {
                Step::Wait(t) => self.wait = t,
                Step::Play { asset, player, voice } => {
                    let Some(key) = self.resolve(&asset, player, voice.as_deref()) else { continue };
                    let dur = a.clip(&key).map_or(0.0, |c| c.data.len() as f32 / c.rate.max(1) as f32);
                    if dur > 0.0 {
                        log::debug!("speech: {} -> {} ({:.1} s)", asset, key, dur);
                        a.play(&key, 1.0);
                        self.wait = dur + 0.15;
                    }
                }
            }
        }
    }
}
