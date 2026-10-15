//! A mock of the GitHub Releases API for driving the real launcher: `mock_server <fixture-dir> <port> [--mode normal|ratelimit|malformed|tampered|slow]`
//! (fixture made by `make_fixture`). Prints every request. Stop it with Ctrl+C or by killing the process.

#[path = "../tests/common/mod.rs"]
mod common;

use common::*;
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = PathBuf::from(args.first().expect("mock_server <fixture-dir> <port>"));
    let port: u16 = args.get(1).expect("port").parse().unwrap();
    let mode = args.iter().position(|a| a == "--mode").and_then(|i| args.get(i + 1)).cloned().unwrap_or_else(|| "normal".into());
    let server = MockServer::start_on(&format!("127.0.0.1:{port}"));
    let latest = std::fs::read(dir.join("latest.json")).unwrap();
    let list = std::fs::read(dir.join("releases.json")).unwrap();
    let etag = format!("\"{}\"", &sha(&latest)[..16]);
    match mode.as_str() {
        "ratelimit" => {
            let reset = mcla_updater::now_unix() + 1500;
            server.set(&format!("/repos/{REPO}/releases/latest"), Route::Status { code: 403, headers: vec![("x-ratelimit-remaining".into(), "0".into()), ("x-ratelimit-reset".into(), reset.to_string())], body: b"{\"message\":\"API rate limit exceeded\"}".to_vec() });
        }
        "malformed" => server.set(&format!("/repos/{REPO}/releases/latest"), Route::Ok { body: b"{ not json".to_vec(), etag: None }),
        _ => server.set(&format!("/repos/{REPO}/releases/latest"), Route::Ok { body: latest, etag: Some(etag) }),
    }
    server.set(&format!("/repos/{REPO}/releases?per_page=20"), Route::Ok { body: list, etag: None });
    if let Ok(rd) = std::fs::read_dir(dir.join("assets")) {
        for tag in rd.flatten() {
            let tag_name = tag.file_name().to_string_lossy().into_owned();
            for f in std::fs::read_dir(tag.path()).unwrap().flatten() {
                let name = f.file_name().to_string_lossy().into_owned();
                let mut body = std::fs::read(f.path()).unwrap();
                if mode == "tampered" && name.ends_with(".zip") {
                    let n = body.len() / 2;
                    body[n] ^= 0xff;
                }
                let route = if mode == "slow" && (name.ends_with(".zip") || name.ends_with(".exe")) { Route::Slow { body, per_sec: 3_000_000 } } else { Route::Ok { body, etag: None } };
                server.set(&format!("/{REPO}/releases/download/{tag_name}/{name}"), route);
            }
        }
    }
    println!("mock GitHub API on {} (mode {mode}), fixture {}", server.base, dir.display());
    let mut seen = 0usize;
    loop {
        std::thread::sleep(std::time::Duration::from_millis(300));
        let n = server.total_hits();
        if n != seen {
            seen = n;
            println!("requests so far: {n}");
        }
    }
}
