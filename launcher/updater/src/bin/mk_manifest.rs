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
//!
//! The release workflow cannot sign (the private key stays offline), so there are two more steps around the signature:
//!
//! * `--unsigned` (instead of a signature file): writes the manifest with `launcher.signature` empty (`manifest.unsigned.json`). Everything
//!   else is validated the usual way, with a placeholder signature standing in.
//! * `--finish <manifest.unsigned.json> --assets <dir> --pubkey <update-key.pub> [--signature <file>] [--out <file>]`: run on the maintainer's
//!   machine after `minisign -S`. It checks that the exe in `<assets>` is the one the unsigned manifest describes (size and SHA-256), that the
//!   signature's trusted comment is `RustyMCLA <launcher version>`, that the signature verifies for that exe with the given public key, embeds it,
//!   validates the result with the launcher's own parser and writes `manifest.json`.

use mcla_updater::config::{valid_repo, ReleaseConfig};
use mcla_updater::http::sha256_file;
use mcla_updater::manifest::{build, BuildInput, ReleaseManifest};
use std::path::PathBuf;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

/// A syntactically valid minisign signature file (not a real signature): stands in for the signature while an unsigned manifest is validated.
const PLACEHOLDER_SIGNATURE: &str = "untrusted comment: signature from minisign secret key\nRUQOFRwjKjE4P0ZNVFtiaXB3foWMk5qhqK+2vcTL0tng5+71AQgPFh0kKzI5QEdOVVxjanF4f4aNlJuiqbC3vsXM09rh6O/2Agk=\ntrusted comment: timestamp:0\nBRIfLDlGU2BteoeUoa67yNXi7wEOGyg1Qk9caXaDkJ2qt8TR3uv4ChckMT5LWGVyf4yZprPAzdrn9AYTIC06Rw==\n";

/// `--finish`: signature + unsigned manifest -> the real manifest.json (see the module comment).
fn finish(args: &[String], unsigned_path: &str) -> Result<(), String> {
    let assets = PathBuf::from(arg(args, "--assets").ok_or("--assets <dir> is required")?);
    let pubkey_path = arg(args, "--pubkey").ok_or("--pubkey <update-key.pub> is required with --finish")?;
    let text = std::fs::read_to_string(unsigned_path).map_err(|e| format!("{unsigned_path}: {e}"))?;
    let mut value: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("{unsigned_path}: not JSON: {e}"))?;
    let launcher = value.get("launcher").cloned().ok_or("the manifest has no launcher entry")?;
    let field = |k: &str| launcher.get(k).and_then(|v| v.as_str()).map(str::to_string).ok_or(format!("launcher.{k} is missing"));
    let (l_version, l_asset, l_sha) = (field("version")?, field("asset")?, field("sha256")?);
    let l_size = launcher.get("size").and_then(|v| v.as_u64()).ok_or("launcher.size is missing")?;
    if !launcher.get("signature").and_then(|v| v.as_str()).unwrap_or("").is_empty() {
        return Err(format!("{unsigned_path} already carries a signature; pass the manifest.unsigned.json of the release"));
    }
    let exe = assets.join(&l_asset);
    let (sha, size) = sha256_file(&exe, None).map_err(|e| e.to_string())?;
    if size != l_size || !sha.eq_ignore_ascii_case(&l_sha) {
        return Err(format!("{} is not the launcher the unsigned manifest describes (file: {size} bytes, sha256 {sha}; manifest: {l_size} bytes, sha256 {l_sha})", exe.display()));
    }
    let sig_path = arg(args, "--signature").map(PathBuf::from).unwrap_or_else(|| assets.join(format!("{l_asset}.minisig")));
    let signature = std::fs::read_to_string(&sig_path).map_err(|e| format!("{}: {e}", sig_path.display()))?;
    let sig = minisign_verify::Signature::decode(&signature).map_err(|e| format!("{}: not a minisign signature file: {e}", sig_path.display()))?;
    let label = mcla_updater::selfupdate::signature_label(&l_version);
    if sig.trusted_comment() != label {
        return Err(format!("the signature's trusted comment is \"{}\" but must be \"{label}\"; sign with: minisign -S -m {l_asset} -t \"{label}\"", sig.trusted_comment()));
    }
    let key_text = std::fs::read_to_string(&pubkey_path).map_err(|e| format!("{pubkey_path}: {e}"))?;
    let pk = minisign_verify::PublicKey::decode(&key_text).map_err(|e| format!("{pubkey_path}: no usable public key ({e}); this is the placeholder or a damaged file"))?;
    {
        use std::io::Read;
        let mut file = std::fs::File::open(&exe).map_err(|e| format!("{}: {e}", exe.display()))?;
        let mut v = pk.verify_stream(&sig).map_err(|e| format!("cannot start verifying: {e}"))?;
        let mut buf = vec![0u8; 1 << 20];
        loop {
            let n = file.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            v.update(&buf[..n]);
        }
        v.finalize().map_err(|e| format!("the signature does not verify for {} with {pubkey_path}: {e}", exe.display()))?;
    }
    value["launcher"]["signature"] = serde_json::Value::String(signature);
    let out_text = serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?;
    let cfg = ReleaseConfig::from_env(env!("CARGO_PKG_VERSION"));
    let m = ReleaseManifest::parse(&out_text, &cfg).map_err(|e| format!("the manifest would not pass the launcher's own validation: {e}"))?;
    let out = arg(args, "--out").map(PathBuf::from).unwrap_or_else(|| assets.join("manifest.json"));
    std::fs::write(&out, format!("{out_text}\n")).map_err(|e| format!("{}: {e}", out.display()))?;
    println!("signature verified with {pubkey_path}; wrote {} (release {}, launcher {} sha256 {})", out.display(), m.version, m.launcher.version, m.launcher.sha256);
    Ok(())
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|a| a == "--help" || a == "-h") {
        println!("usage: mk-manifest --assets <dir> --version 0.2.0 [--launcher-version V] [--game-version V] [--min-launcher V] [--data-version N]\n       [--launcher-asset RustyMCLA.exe] [--game-asset game-<version>.zip] [--signature <file> | --unsigned] [--install-manifest <json>]\n       [--notes-url URL] [--repo owner/name] [--out <file>]\n       mk-manifest --finish <manifest.unsigned.json> --assets <dir> --pubkey <update-key.pub> [--signature <file>] [--out <file>]");
        return Ok(());
    }
    if let Some(unsigned) = arg(&args, "--finish") {
        return finish(&args, &unsigned);
    }
    let unsigned = args.iter().any(|a| a == "--unsigned");
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
    let signature = if unsigned {
        PLACEHOLDER_SIGNATURE.to_string()
    } else {
        let sig_path = arg(&args, "--signature").map(PathBuf::from).unwrap_or_else(|| assets.join(format!("{launcher_asset}.minisig")));
        let signature = std::fs::read_to_string(&sig_path).map_err(|e| format!("{}: {e} (sign the launcher offline with minisign first)", sig_path.display()))?;

        // the signature's trusted comment must name this launcher version (the launcher refuses anything else: rollback protection)
        let label = mcla_updater::selfupdate::signature_label(&launcher_version);
        let sig = minisign_verify::Signature::decode(&signature).map_err(|e| format!("{}: not a minisign signature file: {e}", sig_path.display()))?;
        if sig.trusted_comment() != label {
            return Err(format!("the signature's trusted comment is \"{}\" but must be \"{label}\"; sign with: minisign -S -m {launcher_asset} -t \"{label}\"", sig.trusted_comment()));
        }
        signature
    };
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
    let mut text = serde_json::to_string_pretty(&m).map_err(|e| e.to_string())?;
    ReleaseManifest::parse(&text, &cfg).map_err(|e| format!("the manifest would not pass the launcher's own validation: {e}"))?;
    let default_out = if unsigned { "manifest.unsigned.json" } else { "manifest.json" };
    if unsigned {
        // the placeholder only stood in for validation; the file says honestly that nothing is signed yet
        let mut v: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        v["launcher"]["signature"] = serde_json::Value::String(String::new());
        text = serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?;
    }
    let out = arg(&args, "--out").map(PathBuf::from).unwrap_or_else(|| assets.join(default_out));
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
