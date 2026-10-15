//! Launcher self-update for a portable single exe (no installer, never without the user pressing Update now).
//!
//! 1. The new `RustyMCLA.exe` is downloaded next to the running one as `<exe>.new.part`; size and SHA-256 from the manifest are
//!    checked while it streams.
//! 2. Its minisign signature (text from the manifest) is verified against the **public key compiled into the launcher**
//!    (`launcher/update-key.pub`). No key set, or a signature that does not verify, means no update; the file is deleted.
//! 3. The running exe is renamed to `<exe>.old` (Windows allows renaming a running exe), the new one is renamed into its place, and the
//!    new one is started. If either rename fails the first is undone; if the new exe cannot be started the old one is put back.
//! 4. On the next start `<exe>.old` is deleted ([`cleanup_after_start`]).
//!
//! The SHA-256 in the manifest protects against corruption; the signature, made offline with a key that is never on the build machine,
//! is what makes a tampered release (or a stolen GitHub token) unable to push code into existing launchers.

use crate::config::ReleaseConfig;
use crate::error::{Kind, Result, UpdateError};
use crate::http::{sha256_file, Http};
use crate::manifest::{LauncherAsset, ReleaseManifest};
use crate::releases::Release;
use crate::{Cancel, Phase, Progress, ProgressFn};
use std::path::{Path, PathBuf};

fn with_suffix(exe: &Path, suffix: &str) -> PathBuf {
    let mut s = exe.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

pub fn part_path(exe: &Path) -> PathBuf {
    with_suffix(exe, ".new.part")
}
pub fn new_path(exe: &Path) -> PathBuf {
    with_suffix(exe, ".new")
}
pub fn old_path(exe: &Path) -> PathBuf {
    with_suffix(exe, ".old")
}

/// The embedded public key, or an error saying that none is set (the shipped placeholder file does not decode).
pub fn public_key(cfg: &ReleaseConfig) -> Result<minisign_verify::PublicKey> {
    minisign_verify::PublicKey::decode(&cfg.public_key).map_err(|e| UpdateError::new(Kind::Signature, format!("this launcher has no signing key (launcher/update-key.pub is not set), so it cannot update itself: {e}")))
}

/// What the release signature's trusted comment must say for launcher `version` (sign with `minisign -S -m RustyMCLA.exe -t "RustyMCLA <version>"`).
pub fn signature_label(version: &str) -> String {
    format!("RustyMCLA {version}")
}

/// Checks size, SHA-256 and signature of a downloaded launcher file against its manifest entry.
pub fn verify_file(path: &Path, la: &LauncherAsset, cfg: &ReleaseConfig, cancel: &Cancel) -> Result<()> {
    let pk = public_key(cfg)?;
    let (hash, size) = sha256_file(path, Some(cancel))?;
    if size != la.size {
        return Err(UpdateError::new(Kind::Size, format!("the launcher file is {size} bytes, the manifest says {}", la.size)));
    }
    if !hash.eq_ignore_ascii_case(&la.sha256) {
        return Err(UpdateError::new(Kind::Hash, format!("launcher sha256 is {hash}, the manifest says {}", la.sha256)));
    }
    let sig = minisign_verify::Signature::decode(&la.signature).map_err(|e| UpdateError::new(Kind::Signature, format!("signature does not decode: {e}")))?;
    // The signed trusted comment names the version, so an old signed exe cannot be offered as a newer release (rollback).
    let want = signature_label(&la.version);
    if sig.trusted_comment() != want {
        return Err(UpdateError::new(Kind::Signature, format!("the signature is for \"{}\", this release needs \"{want}\"", sig.trusted_comment().chars().take(60).collect::<String>())));
    }
    let mut file = std::fs::File::open(path).map_err(|e| UpdateError::io(path.display(), &e))?;
    let mut v = pk.verify_stream(&sig).map_err(|e| UpdateError::new(Kind::Signature, format!("cannot start verifying: {e}")))?;
    let mut buf = vec![0u8; 1 << 20];
    loop {
        use std::io::Read;
        let n = file.read(&mut buf).map_err(|e| UpdateError::io(path.display(), &e))?;
        if n == 0 {
            break;
        }
        v.update(&buf[..n]);
    }
    v.finalize().map_err(|e| UpdateError::new(Kind::Signature, format!("the signature does not match this file and the launcher's key: {e}")))
}

/// Downloads and verifies the new launcher; returns the staged file (`<exe>.new`). The running exe is not touched.
pub async fn download_and_stage(http: &Http, release: &Release, m: &ReleaseManifest, exe: &Path, cancel: &Cancel, progress: &ProgressFn) -> Result<PathBuf> {
    public_key(&http.cfg)?; // refuse before downloading anything when no key is set
    let la = &m.launcher;
    let asset = release.asset(&la.asset).ok_or_else(|| UpdateError::new(Kind::Missing, format!("release {} has no {}", release.tag, la.asset)))?;
    if asset.size != 0 && asset.size != la.size {
        return Err(UpdateError::new(Kind::Size, format!("the release lists {} as {} bytes, the manifest says {}", asset.name, asset.size, la.size)));
    }
    let (part, staged) = (part_path(exe), new_path(exe));
    let p = progress.clone();
    http.download(&asset.url, &part, la.size, &la.sha256, cancel, &move |d, t| p(Progress { phase: Phase::DownloadLauncher, done: d, total: t })).await?;
    progress(Progress { phase: Phase::VerifyLauncher, done: 0, total: 1 });
    let (part2, la2, cfg2, cancel2) = (part.clone(), la.clone(), http.cfg.clone(), cancel.clone());
    let v = tokio::task::spawn_blocking(move || verify_file(&part2, &la2, &cfg2, &cancel2)).await.unwrap_or_else(|e| Err(UpdateError::new(Kind::Io, format!("verify thread failed: {e}"))));
    if let Err(e) = v {
        let _ = std::fs::remove_file(&part);
        return Err(e);
    }
    let _ = std::fs::remove_file(&staged);
    std::fs::rename(&part, &staged).map_err(|e| {
        let _ = std::fs::remove_file(&part);
        UpdateError::io(staged.display(), &e)
    })?;
    progress(Progress { phase: Phase::VerifyLauncher, done: 1, total: 1 });
    Ok(staged)
}

/// Swaps the staged file in: `exe` becomes `exe.old`, `staged` becomes `exe`. Returns the `.old` path. Undone if the second step fails.
pub fn apply(exe: &Path, staged: &Path) -> Result<PathBuf> {
    let old = old_path(exe);
    if old.exists() {
        std::fs::remove_file(&old).map_err(|e| UpdateError::io(format!("cannot remove the previous {}", old.display()), &e))?;
    }
    std::fs::rename(exe, &old).map_err(|e| UpdateError::io(format!("cannot move the running launcher aside ({})", exe.display()), &e))?;
    if let Err(e) = std::fs::rename(staged, exe) {
        if let Err(e2) = std::fs::rename(&old, exe) {
            log::error!("self-update rollback failed: {e2}");
        }
        return Err(UpdateError::io(format!("cannot put the new launcher in place ({})", exe.display()), &e));
    }
    Ok(old)
}

/// Undoes [`apply`] (the new exe could not be started): the new file is deleted and the old one is back.
pub fn rollback(exe: &Path, old: &Path) -> Result<()> {
    let _ = std::fs::remove_file(exe);
    std::fs::rename(old, exe).map_err(|e| UpdateError::io(format!("cannot restore {}", old.display()), &e))
}

/// Starts the (new) exe. It is passed `--updated` so it knows to log the update and clean up.
pub fn restart(exe: &Path) -> std::io::Result<std::process::Child> {
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("--updated");
    // test hooks of the old process must not drive the new one
    for v in ["RUSTY_MCLA_SELFTEST", "RUSTY_MCLA_UPDATE_TEST", "RUSTY_MCLA_INSTALL_TEST"] {
        cmd.env_remove(v);
    }
    if let Some(dir) = exe.parent() {
        cmd.current_dir(dir);
    }
    cmd.spawn()
}

/// At every start: removes `<exe>.old` (left by the update that started us) and partial downloads. Returns whether an `.old` was there.
pub fn cleanup_after_start(exe: &Path) -> bool {
    let mut had_old = false;
    let old = old_path(exe);
    if old.exists() {
        had_old = true;
        // the previous process may still be exiting: try a few times
        for _ in 0..20 {
            if std::fs::remove_file(&old).is_ok() {
                log::info!("removed {}", old.display());
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
    }
    for p in [part_path(exe), new_path(exe)] {
        if p.exists() {
            let _ = std::fs::remove_file(&p);
        }
    }
    had_old
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::manifest::tests::sample;

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
            minisign::sign(Some(&self.pk), &self.sk, std::io::Cursor::new(data), Some(&super::signature_label(version)), Some("test signature")).unwrap().to_string()
        }
    }

    fn asset_for(data: &[u8], key: &TestKey) -> LauncherAsset {
        use sha2::{Digest, Sha256};
        let mut m = sample().launcher;
        m.size = data.len() as u64;
        m.sha256 = crate::hex(&Sha256::digest(data));
        m.signature = key.sign(data, &m.version);
        m
    }

    fn cfg_with(key: &TestKey) -> ReleaseConfig {
        let mut c = ReleaseConfig::embedded("0.1.0");
        c.public_key = key.public_text.clone();
        c
    }

    const FILE: &[u8] = b"MZ pretend this is the new launcher exe";

    #[test]
    fn a_correctly_signed_file_verifies() {
        let (key, d) = (new_key(), tempfile::tempdir().unwrap());
        let f = d.path().join("new.exe");
        std::fs::write(&f, FILE).unwrap();
        verify_file(&f, &asset_for(FILE, &key), &cfg_with(&key), &Cancel::new()).unwrap();
    }

    #[test]
    fn a_tampered_file_is_refused_by_hash_and_by_signature() {
        let (key, d) = (new_key(), tempfile::tempdir().unwrap());
        let f = d.path().join("new.exe");
        std::fs::write(&f, b"MZ pretend this is the NEW launcher exe").unwrap(); // same length, other bytes
        let la = asset_for(FILE, &key);
        assert_eq!(verify_file(&f, &la, &cfg_with(&key), &Cancel::new()).unwrap_err().kind, Kind::Hash);
        // a manifest with the right hash for the tampered file but the old signature: the signature catches it
        let mut la2 = la.clone();
        la2.sha256 = crate::http::sha256_file(&f, None).unwrap().0;
        assert_eq!(verify_file(&f, &la2, &cfg_with(&key), &Cancel::new()).unwrap_err().kind, Kind::Signature);
    }

    #[test]
    fn a_signature_by_another_key_is_refused() {
        let (key, attacker, d) = (new_key(), new_key(), tempfile::tempdir().unwrap());
        let f = d.path().join("new.exe");
        std::fs::write(&f, FILE).unwrap();
        let la = asset_for(FILE, &attacker);
        assert_eq!(verify_file(&f, &la, &cfg_with(&key), &Cancel::new()).unwrap_err().kind, Kind::Signature);
    }

    #[test]
    fn a_placeholder_key_means_no_self_update() {
        let (key, d) = (new_key(), tempfile::tempdir().unwrap());
        let f = d.path().join("new.exe");
        std::fs::write(&f, FILE).unwrap();
        let mut cfg = ReleaseConfig::embedded("0.1.0");
        cfg.public_key = "untrusted comment: NOT SET. Replace this whole file with the public key made offline by `minisign -G`.\n".to_string();
        let e = verify_file(&f, &asset_for(FILE, &key), &cfg, &Cancel::new()).unwrap_err();
        assert_eq!(e.kind, Kind::Signature);
        assert!(e.detail.contains("no signing key"));
    }

    #[test]
    fn the_shipped_public_key_is_a_real_minisign_key() {
        // launcher/update-key.pub holds the maintainer's public key (made offline on 2026-10-15); it must decode, and a signature by any other key must not verify with it
        let cfg = ReleaseConfig::embedded("0.1.0");
        assert!(minisign_verify::PublicKey::decode(&cfg.public_key).is_ok(), "the shipped update-key.pub must be a valid minisign public key");
        assert!(cfg.public_key.contains("0B74E78DA50B7E18"));
        let (other, d) = (new_key(), tempfile::tempdir().unwrap());
        let f = d.path().join("new.exe");
        std::fs::write(&f, FILE).unwrap();
        assert_eq!(verify_file(&f, &asset_for(FILE, &other), &cfg, &Cancel::new()).unwrap_err().kind, Kind::Signature);
    }

    #[test]
    fn an_old_signed_exe_cannot_pass_as_a_newer_release() {
        // rollback: the file and its signature are genuine, but the signature was made for version 0.1.0 and the manifest claims 0.2.0
        let (key, d) = (new_key(), tempfile::tempdir().unwrap());
        let f = d.path().join("new.exe");
        std::fs::write(&f, FILE).unwrap();
        let mut la = asset_for(FILE, &key);
        la.signature = key.sign(FILE, "0.1.0");
        let e = verify_file(&f, &la, &cfg_with(&key), &Cancel::new()).unwrap_err();
        assert_eq!(e.kind, Kind::Signature);
        assert!(e.detail.contains("RustyMCLA 0.1.0"), "{}", e.detail);
    }

    #[test]
    fn a_wrong_size_is_refused() {
        let (key, d) = (new_key(), tempfile::tempdir().unwrap());
        let f = d.path().join("new.exe");
        std::fs::write(&f, &FILE[..10]).unwrap();
        assert_eq!(verify_file(&f, &asset_for(FILE, &key), &cfg_with(&key), &Cancel::new()).unwrap_err().kind, Kind::Size);
    }

    #[test]
    fn apply_swaps_and_cleanup_removes_the_old_exe() {
        let d = tempfile::tempdir().unwrap();
        let exe = d.path().join("RustyMCLA.exe");
        std::fs::write(&exe, "OLD").unwrap();
        std::fs::write(new_path(&exe), "NEW").unwrap();
        let old = apply(&exe, &new_path(&exe)).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"NEW");
        assert_eq!(std::fs::read(&old).unwrap(), b"OLD");
        assert!(!new_path(&exe).exists());
        assert!(cleanup_after_start(&exe));
        assert!(!old.exists());
        assert!(!cleanup_after_start(&exe));
    }

    #[test]
    fn apply_rolls_back_when_the_staged_file_is_missing() {
        let d = tempfile::tempdir().unwrap();
        let exe = d.path().join("RustyMCLA.exe");
        std::fs::write(&exe, "OLD").unwrap();
        let e = apply(&exe, &new_path(&exe)).unwrap_err();
        assert_eq!(e.kind, Kind::Io);
        assert_eq!(std::fs::read(&exe).unwrap(), b"OLD");
        assert!(!old_path(&exe).exists());
    }

    #[test]
    fn rollback_restores_the_old_exe() {
        let d = tempfile::tempdir().unwrap();
        let exe = d.path().join("RustyMCLA.exe");
        std::fs::write(&exe, "OLD").unwrap();
        std::fs::write(new_path(&exe), "NEW").unwrap();
        let old = apply(&exe, &new_path(&exe)).unwrap();
        rollback(&exe, &old).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"OLD");
        assert!(!old.exists());
    }

    #[test]
    fn cleanup_removes_partial_downloads() {
        let d = tempfile::tempdir().unwrap();
        let exe = d.path().join("RustyMCLA.exe");
        std::fs::write(&exe, "X").unwrap();
        std::fs::write(part_path(&exe), "p").unwrap();
        std::fs::write(new_path(&exe), "n").unwrap();
        cleanup_after_start(&exe);
        assert!(!part_path(&exe).exists() && !new_path(&exe).exists());
        assert!(exe.exists());
    }
}
