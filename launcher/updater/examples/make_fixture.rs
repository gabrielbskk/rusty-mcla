//! Builds a mock release into a folder for `mock_server` (UI checks against the mock; nothing here touches the network):
//!
//! ```text
//! make_fixture <out-dir> --base http://127.0.0.1:PORT --release 0.2.0 [--launcher-version V] [--game-version V] [--data-version N]
//!              [--launcher-exe <file>] [--older 0.1.0] [--notes <markdown file>]
//! ```
//! Writes `latest.json`, `releases.json` (the release and, with `--older`, one earlier release), `assets/<tag>/...`, `key.pub` (the ephemeral
//! test public key; start the launcher with `RUSTY_MCLA_TEST_PUBKEY=<out-dir>/key.pub`) and `manifest.json` (a copy for the offline package test).
//! `--launcher-exe` serves that file plus 16 bytes of padding as the new launcher (still a runnable exe), signed with the ephemeral key.

#[path = "../tests/common/mod.rs"]
mod common;

use common::*;
use std::path::PathBuf;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = PathBuf::from(args.first().expect("make_fixture <out-dir> --base URL --release V"));
    let base = arg(&args, "--base").expect("--base");
    let release = arg(&args, "--release").expect("--release");
    let lv = arg(&args, "--launcher-version").unwrap_or_else(|| release.clone());
    let gv = arg(&args, "--game-version").unwrap_or_else(|| release.clone());
    let dv: u32 = arg(&args, "--data-version").map(|s| s.parse().unwrap()).unwrap_or(1);
    let notes = arg(&args, "--notes").map(|p| std::fs::read_to_string(p).unwrap()).unwrap_or_else(|| {
        "## Added\n- Update prompts you can always skip.\n- Release notes on the Home page, rendered from the GitHub release body.\n\n## Fixed\n- The launcher starts with no network.\n\n## Known issues\n- Export Logs is not built yet.\n\nSee the [full changelog](https://github.com/OWNER/rusty-mcla/releases).\n\n<script>alert('not run')</script>\n![remote](https://evil.example/pixel.png)".to_string()
    });
    let key = new_key();
    let launcher_bytes = arg(&args, "--launcher-exe").map(|p| {
        let mut b = std::fs::read(p).unwrap();
        b.extend_from_slice(b"RUSTY-MCLA-NEW!!");
        b
    });
    std::fs::create_dir_all(&out).unwrap();
    let fx = fixture_with(&base, &release, &lv, &gv, &key, dv, &notes, launcher_bytes);
    let mut list = vec![fx.release_json.clone()];
    let write_assets = |f: &Fixture| {
        let d = out.join("assets").join(&f.tag);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("manifest.json"), &f.manifest_json).unwrap();
        std::fs::write(d.join(format!("game-{}.zip", f.manifest.game.version)), &f.zip).unwrap();
        std::fs::write(d.join("RustyMCLA.exe"), &f.launcher).unwrap();
        std::fs::write(d.join("RustyMCLA.exe.minisig"), &f.manifest.launcher.signature).unwrap();
    };
    write_assets(&fx);
    if let Some(older) = arg(&args, "--older") {
        let o = fixture_with(&base, &older, &older, &older, &key, 1, "## Added\n- The first public build.", None);
        write_assets(&o);
        list.push(o.release_json.clone());
    }
    std::fs::write(out.join("latest.json"), &fx.release_json).unwrap();
    std::fs::write(out.join("releases.json"), format!("[{}]", list.join(","))).unwrap();
    std::fs::write(out.join("key.pub"), &key.public_text).unwrap();
    std::fs::write(out.join("manifest.json"), &fx.manifest_json).unwrap();
    println!("fixture for release {release} (launcher {lv}, game {gv}, data_version {dv}) written to {}", out.display());
    println!("launcher {} bytes sha256 {}; game zip {} bytes sha256 {}", fx.launcher.len(), fx.manifest.launcher.sha256, fx.zip.len(), fx.manifest.game.sha256);
}
