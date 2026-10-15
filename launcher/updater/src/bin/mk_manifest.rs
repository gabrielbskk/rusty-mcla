//! `mk-manifest`: produces a release's `manifest.json` from a folder of release assets (sizes and SHA-256 are measured, the minisign
//! signature file is embedded, the install parts come from the compiled-in install manifest or a file) and validates it with the same
//! strict parser the launcher uses before writing it. The CI release workflow (a later session) calls this.
//!
//! ```text
//! mk-manifest --assets <dir> --version 0.2.0 [--launcher-version V] [--game-version V] [--min-launcher V] [--data-version N]
//!             [--launcher-asset RustyMCLA.exe] [--game-asset game-<version>.zip] [--signature <file>]
//!             [--install-manifest <json>] [--notes-url URL] [--repo owner/name] [--out <file>]
//! ```
//! `--signature` defaults to `<assets>/<launcher-asset>.minisig`; `--out` to `<assets>/manifest.json`; `--data-version` to the compiled-in one.

use mcla_updater::config::{valid_repo, ReleaseConfig};
use mcla_updater::http::sha256_file;
use mcla_updater::manifest::{build, BuildInput, ReleaseManifest};
use std::path::PathBuf;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|a| a == "--help" || a == "-h") {
        println!("usage: mk-manifest --assets <dir> --version 0.2.0 [--launcher-version V] [--game-version V] [--min-launcher V] [--data-version N]\n       [--launcher-asset RustyMCLA.exe] [--game-asset game-<version>.zip] [--signature <file>] [--install-manifest <json>]\n       [--notes-url URL] [--repo owner/name] [--out <file>]");
        return Ok(());
    }
    let assets = PathBuf::from(arg(&args, "--assets").ok_or("--assets <dir> is required")?);
    let version = arg(&args, "--version").ok_or("--version is required")?;
    let launcher_version = arg(&args, "--launcher-version").unwrap_or_else(|| version.clone());
    let game_version = arg(&args, "--game-version").unwrap_or_else(|| version.clone());
    let min_launcher = arg(&args, "--min-launcher").unwrap_or_else(|| "0.1.0".into());
    let launcher_asset = arg(&args, "--launcher-asset").unwrap_or_else(|| "RustyMCLA.exe".into());
    let game_asset = arg(&args, "--game-asset").unwrap_or_else(|| format!("game-{game_version}.zip"));

    let mut cfg = ReleaseConfig::from_env(env!("CARGO_PKG_VERSION"));
    if let Some(r) = arg(&args, "--repo") {
        if !valid_repo(&r) {
            return Err(format!("--repo {r}: expected owner/name"));
        }
        cfg.repo = r;
    }
    let install = match arg(&args, "--install-manifest") {
        Some(p) => mcla_install::Manifest::load(std::path::Path::new(&p)).map_err(|e| format!("{p}: {e}"))?,
        None => mcla_install::Manifest::embedded(),
    };
    let data_version = match arg(&args, "--data-version") {
        Some(s) => s.parse::<u32>().map_err(|_| "--data-version must be a number".to_string())?,
        None => install.data_version,
    };
    let sig_path = arg(&args, "--signature").map(PathBuf::from).unwrap_or_else(|| assets.join(format!("{launcher_asset}.minisig")));
    let signature = std::fs::read_to_string(&sig_path).map_err(|e| format!("{}: {e} (sign the launcher offline with minisign first)", sig_path.display()))?;

    // the signature's trusted comment must name this launcher version (the launcher refuses anything else: rollback protection)
    let label = mcla_updater::selfupdate::signature_label(&launcher_version);
    let sig = minisign_verify::Signature::decode(&signature).map_err(|e| format!("{}: not a minisign signature file: {e}", sig_path.display()))?;
    if sig.trusted_comment() != label {
        return Err(format!("the signature's trusted comment is \"{}\" but must be \"{label}\"; sign with: minisign -S -m {launcher_asset} -t \"{label}\"", sig.trusted_comment()));
    }
    let (l_sha, l_size) =sha256_file(&assets.join(&launcher_asset), None).map_err(|e| e.to_string())?;
    let (g_sha, g_size) = sha256_file(&assets.join(&game_asset), None).map_err(|e| e.to_string())?;
    let notes_url = arg(&args, "--notes-url").unwrap_or_else(|| cfg.web_release_url(&format!("v{version}")));

    let m = build(
        BuildInput {
            version,
            launcher_version,
            game_version,
            min_launcher_version: min_launcher,
            data_version,
            notes_url,
            launcher_asset,
            launcher_size: l_size,
            launcher_sha256: l_sha,
            launcher_signature: signature,
            game_asset,
            game_size: g_size,
            game_sha256: g_sha,
        },
        &install,
    );
    let text = serde_json::to_string_pretty(&m).map_err(|e| e.to_string())?;
    ReleaseManifest::parse(&text, &cfg).map_err(|e| format!("the manifest would not pass the launcher's own validation: {e}"))?;
    let out = arg(&args, "--out").map(PathBuf::from).unwrap_or_else(|| assets.join("manifest.json"));
    std::fs::write(&out, format!("{text}\n")).map_err(|e| format!("{}: {e}", out.display()))?;
    println!("wrote {} (release {}, launcher {} bytes, game {} bytes, data_version {})", out.display(), m.version, m.launcher.size, m.game.size, m.game.data_version);
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("mk-manifest: {e}");
        std::process::exit(1);
    }
}
