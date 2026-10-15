//! A tiny mock of the GitHub Releases API and the asset downloads, shared by the integration tests and the `mock_server` example.
#![allow(dead_code)]

use mcla_updater::manifest::{build, BuildInput, ReleaseManifest};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{Cursor, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

#[derive(Clone, Debug)]
pub enum Route {
    /// 200 with a body; with an ETag the server answers 304 to a matching `If-None-Match`.
    Ok { body: Vec<u8>, etag: Option<String> },
    Status { code: u16, headers: Vec<(String, String)>, body: Vec<u8> },
    Redirect(String),
    /// Announces `declared` bytes but sends only `sent` and closes (a cut connection or a short file).
    Cut { declared: usize, sent: Vec<u8> },
    /// 200 with a body sent at most `per_sec` bytes per second.
    Slow { body: Vec<u8>, per_sec: usize },
}

#[derive(Clone, Debug)]
pub struct Hit {
    pub path: String,
    pub if_none_match: Option<String>,
}

#[derive(Default)]
struct State {
    routes: HashMap<String, Route>,
    hits: Vec<Hit>,
}

pub struct MockServer {
    pub base: String,
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

fn header(name: &str, value: &str) -> tiny_http::Header {
    tiny_http::Header::from_bytes(name.as_bytes(), value.as_bytes()).unwrap()
}

impl MockServer {
    pub fn start() -> MockServer {
        Self::start_on("127.0.0.1:0")
    }

    pub fn start_on(addr: &str) -> MockServer {
        let server = tiny_http::Server::http(addr).expect("bind the mock server");
        let port = server.server_addr().to_ip().expect("ip address").port();
        let state = Arc::new(Mutex::new(State::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (st, sp) = (state.clone(), stop.clone());
        let handle = std::thread::spawn(move || {
            while !sp.load(Ordering::SeqCst) {
                let Ok(Some(req)) = server.recv_timeout(Duration::from_millis(50)) else { continue };
                let url = req.url().to_string();
                let inm = req.headers().iter().find(|h| h.field.equiv("If-None-Match")).map(|h| h.value.to_string());
                let route = {
                    let mut s = st.lock().unwrap();
                    s.hits.push(Hit { path: url.clone(), if_none_match: inm.clone() });
                    s.routes.get(&url).cloned()
                };
                match route {
                    None => {
                        let _ = req.respond(tiny_http::Response::from_string("not found").with_status_code(404));
                    }
                    Some(Route::Ok { body, etag }) => {
                        if let (Some(e), Some(got)) = (&etag, &inm) {
                            if e == got {
                                let _ = req.respond(tiny_http::Response::empty(304).with_header(header("ETag", e)));
                                continue;
                            }
                        }
                        let mut r = tiny_http::Response::from_data(body);
                        if let Some(e) = etag {
                            r = r.with_header(header("ETag", &e));
                        }
                        let _ = req.respond(r);
                    }
                    Some(Route::Status { code, headers, body }) => {
                        let mut r = tiny_http::Response::from_data(body).with_status_code(code);
                        for (k, v) in headers {
                            r = r.with_header(header(&k, &v));
                        }
                        let _ = req.respond(r);
                    }
                    Some(Route::Redirect(to)) => {
                        let _ = req.respond(tiny_http::Response::empty(302).with_header(header("Location", &to)));
                    }
                    Some(Route::Slow { body, per_sec }) => {
                        let n = body.len();
                        let r = tiny_http::Response::new(tiny_http::StatusCode(200), vec![], Slow { data: Cursor::new(body), per_sec }, Some(n), None);
                        let _ = req.respond(r);
                    }
                    Some(Route::Cut { declared, sent }) => {
                        let r = tiny_http::Response::new(tiny_http::StatusCode(200), vec![], Cursor::new(sent), Some(declared), None);
                        let _ = req.respond(r);
                    }
                }
            }
        });
        MockServer { base: format!("http://127.0.0.1:{port}"), state, stop, handle: Some(handle) }
    }

    pub fn set(&self, path: &str, route: Route) {
        self.state.lock().unwrap().routes.insert(path.to_string(), route);
    }

    pub fn remove(&self, path: &str) {
        self.state.lock().unwrap().routes.remove(path);
    }

    pub fn hits(&self, path: &str) -> Vec<Hit> {
        self.state.lock().unwrap().hits.iter().filter(|h| h.path == path).cloned().collect()
    }

    pub fn clear_hits(&self) {
        self.state.lock().unwrap().hits.clear();
    }

    pub fn total_hits(&self) -> usize {
        self.state.lock().unwrap().hits.len()
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

// ---------- signing (ephemeral test keys only) ----------

pub struct TestKey {
    pub public_text: String,
    sk: minisign::SecretKey,
    pk: minisign::PublicKey,
}

pub fn new_key() -> TestKey {
    let kp = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
    TestKey { public_text: kp.pk.to_box().unwrap().to_string(), sk: kp.sk, pk: kp.pk }
}

impl TestKey {
    pub fn sign(&self, data: &[u8], version: &str) -> String {
        minisign::sign(Some(&self.pk), &self.sk, Cursor::new(data), Some(&mcla_updater::selfupdate::signature_label(version)), Some("test signature")).unwrap().to_string()
    }
}

// ---------- a release fixture ----------

pub fn noise(n: usize, seed: u32) -> Vec<u8> {
    (0..n as u32).map(|i| (i.wrapping_add(seed).wrapping_mul(2654435761) >> 13) as u8).collect()
}

pub fn sha(data: &[u8]) -> String {
    mcla_updater::hex(&Sha256::digest(data))
}

pub fn make_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opt = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, data) in entries {
        w.start_file(*name, opt).unwrap();
        w.write_all(data).unwrap();
    }
    w.finish().unwrap().into_inner()
}

pub struct Fixture {
    pub version: String,
    pub tag: String,
    pub launcher: Vec<u8>,
    pub zip: Vec<u8>,
    pub manifest: ReleaseManifest,
    pub manifest_json: String,
    pub release_json: String,
    pub paths: Paths,
}

pub struct Paths {
    pub latest: String,
    pub manifest: String,
    pub zip: String,
    pub launcher: String,
}

pub const REPO: &str = "OWNER/rusty-mcla";

/// A complete release `version` for a mock at `base`: launcher exe (signed with `key`), game zip, manifest, release JSON.
pub fn fixture(base: &str, version: &str, key: &TestKey, data_version: u32, body: &str) -> Fixture {
    fixture_with(base, version, version, version, key, data_version, body, None)
}

/// Like [`fixture`] with separate launcher and game versions and (optionally) the bytes of the launcher asset.
#[allow(clippy::too_many_arguments)]
pub fn fixture_with(base: &str, version: &str, launcher_version: &str, game_version: &str, key: &TestKey, data_version: u32, body: &str, launcher_bytes: Option<Vec<u8>>) -> Fixture {
    let tag = format!("v{version}");
    let launcher = launcher_bytes.unwrap_or_else(|| {
        let mut v = b"MZ".to_vec();
        v.extend(noise(150_000, version.len() as u32 + data_version));
        v
    });
    let game_exe = format!("MZ game {game_version}").into_bytes();
    // MOCK_GAME_MB=N pads the game package with N MB of incompressible data (to watch a progress bar)
    let pad = std::env::var("MOCK_GAME_MB").ok().and_then(|s| s.parse::<usize>().ok()).unwrap_or(0) * 1_000_000;
    let big = rand_bytes(pad, 0x9E37_79B9_7F4A_7C15);
    let zip = make_zip(&[("mcla.exe", &game_exe), ("data/shaders.bin", &noise(20_000, 7)), ("readme.txt", b"hello"), ("data/big.bin", &big)]);
    let dl = |name: &str| format!("{base}/{REPO}/releases/download/{tag}/{name}");
    let manifest = build(
        BuildInput {
            version: version.into(),
            launcher_version: launcher_version.into(),
            game_version: game_version.into(),
            min_launcher_version: "0.1.0".into(),
            data_version,
            notes_url: format!("{base}/{REPO}/releases/tag/{tag}"),
            launcher_asset: "RustyMCLA.exe".into(),
            launcher_size: launcher.len() as u64,
            launcher_sha256: sha(&launcher),
            launcher_signature: key.sign(&launcher, launcher_version),
            game_asset: format!("game-{game_version}.zip"),
            game_size: zip.len() as u64,
            game_sha256: sha(&zip),
        },
        &mcla_install::Manifest::embedded(),
    );
    let manifest_json = serde_json::to_string_pretty(&manifest).unwrap();
    let release_json = serde_json::json!({
        "tag_name": tag, "name": format!("Rusty MCLA {version}"), "body": body, "published_at": "2026-10-15T10:00:00Z",
        "html_url": format!("{base}/{REPO}/releases/tag/{tag}"), "draft": false, "prerelease": false,
        "assets": [
            {"name": "manifest.json", "size": manifest_json.len(), "browser_download_url": dl("manifest.json")},
            {"name": format!("game-{game_version}.zip"), "size": zip.len(), "browser_download_url": dl(&format!("game-{game_version}.zip"))},
            {"name": "RustyMCLA.exe", "size": launcher.len(), "browser_download_url": dl("RustyMCLA.exe")},
        ]
    })
    .to_string();
    let paths = Paths { latest: format!("/repos/{REPO}/releases/latest"), manifest: format!("/{REPO}/releases/download/{tag}/manifest.json"), zip: format!("/{REPO}/releases/download/{tag}/game-{game_version}.zip"), launcher: format!("/{REPO}/releases/download/{tag}/RustyMCLA.exe") };
    Fixture { version: version.into(), tag, launcher, zip, manifest, manifest_json, release_json, paths }
}

impl Fixture {
    /// Serves this release as the latest one (ETag = `etag`).
    pub fn publish(&self, mock: &MockServer, etag: &str) {
        mock.set(&self.paths.latest, Route::Ok { body: self.release_json.clone().into_bytes(), etag: Some(etag.to_string()) });
        mock.set(&self.paths.manifest, Route::Ok { body: self.manifest_json.clone().into_bytes(), etag: None });
        mock.set(&self.paths.zip, Route::Ok { body: self.zip.clone(), etag: None });
        mock.set(&self.paths.launcher, Route::Ok { body: self.launcher.clone(), etag: None });
    }
}

/// A reader that hands out `bytes_per_sec` at most (for watching a progress bar).
pub struct Slow {
    data: Cursor<Vec<u8>>,
    per_sec: usize,
}

impl std::io::Read for Slow {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.data.read(buf)?;
        std::thread::sleep(Duration::from_secs_f64(n as f64 / self.per_sec.max(1) as f64));
        Ok(n)
    }
}

/// Incompressible bytes (xorshift), so a zip keeps its size.
pub fn rand_bytes(n: usize, mut x: u64) -> Vec<u8> {
    let mut v = Vec::with_capacity(n);
    while v.len() < n {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        v.extend_from_slice(&x.to_le_bytes());
    }
    v.truncate(n);
    v
}
