//! The two-step release manifest used by the release workflow and `tools/sign-release.ps1`:
//! `mk-manifest --unsigned` (CI, no key) and `mk-manifest --finish` (the maintainer's machine, after `minisign -S`).
//! Ephemeral test keys only.

mod common;

use common::{make_zip, new_key, noise};
use std::path::Path;
use std::process::{Command, Output};

const VERSION: &str = "0.3.0";

fn mk(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mk-manifest")).args(args).output().unwrap()
}

fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

/// A folder with RustyMCLA.exe and game-0.3.0.zip, and the unsigned manifest made from them.
fn draft() -> (tempfile::TempDir, Vec<u8>) {
    let d = tempfile::tempdir().unwrap();
    let exe = noise(200_000, 7);
    std::fs::write(d.path().join("RustyMCLA.exe"), &exe).unwrap();
    std::fs::write(d.path().join(format!("game-{VERSION}.zip")), make_zip(&[("mcla.exe", &noise(4000, 1)), ("LICENSE", b"gpl")])).unwrap();
    let o = mk(&["--assets", s(d.path()), "--version", VERSION, "--unsigned", "--repo", "someone/rusty-mcla"]);
    assert!(o.status.success(), "{}", text(&o));
    (d, exe)
}

fn write_sig(d: &Path, key: &common::TestKey, exe: &[u8], version: &str) {
    std::fs::write(d.join("RustyMCLA.exe.minisig"), key.sign(exe, version)).unwrap();
}

#[test]
fn unsigned_manifest_has_an_empty_signature_and_everything_else() {
    let (d, exe) = draft();
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(d.path().join("manifest.unsigned.json")).unwrap()).unwrap();
    assert_eq!(v["launcher"]["signature"], "");
    assert_eq!(v["launcher"]["version"], VERSION);
    assert_eq!(v["launcher"]["size"], exe.len());
    assert_eq!(v["game"]["asset"], format!("game-{VERSION}.zip"));
    assert_eq!(v["notes_url"], format!("https://github.com/someone/rusty-mcla/releases/tag/v{VERSION}"));
    assert!(!d.path().join("manifest.json").exists(), "an unsigned run must not write the real manifest");
}

#[test]
fn finishing_embeds_the_signature_and_the_launcher_accepts_the_result() {
    let (d, exe) = draft();
    let key = new_key();
    write_sig(d.path(), &key, &exe, VERSION);
    let pubkey = d.path().join("key.pub");
    std::fs::write(&pubkey, &key.public_text).unwrap();
    let o = mk(&["--finish", s(&d.path().join("manifest.unsigned.json")), "--assets", s(d.path()), "--pubkey", s(&pubkey)]);
    assert!(o.status.success(), "{}", text(&o));
    let signed = std::fs::read_to_string(d.path().join("manifest.json")).unwrap();
    let unsigned: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(d.path().join("manifest.unsigned.json")).unwrap()).unwrap();
    let mut v: serde_json::Value = serde_json::from_str(&signed).unwrap();
    assert!(v["launcher"]["signature"].as_str().unwrap().contains("trusted comment: RustyMCLA 0.3.0"));
    // only the signature differs from the unsigned file
    v["launcher"]["signature"] = "".into();
    assert_eq!(v, unsigned);
    // the launcher's own strict parser takes it
    let cfg = mcla_updater::config::ReleaseConfig::embedded("0.1.0");
    mcla_updater::manifest::ReleaseManifest::parse(&signed, &cfg).unwrap();
    // and a signed run of the one-step form produces the same manifest
    let o = mk(&["--assets", s(d.path()), "--version", VERSION, "--repo", "someone/rusty-mcla", "--out", s(&d.path().join("one-step.json"))]);
    assert!(o.status.success(), "{}", text(&o));
    let one_step: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(d.path().join("one-step.json")).unwrap()).unwrap();
    assert_eq!(one_step, serde_json::from_str::<serde_json::Value>(&signed).unwrap());
}

#[test]
fn finishing_refuses_what_must_not_be_signed_or_shipped() {
    let (d, exe) = draft();
    let (key, other) = (new_key(), new_key());
    let unsigned = d.path().join("manifest.unsigned.json");
    let pubkey = d.path().join("key.pub");
    std::fs::write(&pubkey, &key.public_text).unwrap();
    let finish = |extra: &[&str]| {
        let mut a = vec!["--finish", s(&unsigned), "--assets", s(d.path()), "--pubkey", s(&pubkey)];
        a.extend_from_slice(extra);
        mk(&a)
    };

    // a signature of another key
    write_sig(d.path(), &other, &exe, VERSION);
    let o = finish(&[]);
    assert!(!o.status.success() && text(&o).contains("different key"), "{}", text(&o));

    // a signature made for another version (rollback label)
    write_sig(d.path(), &key, &exe, "0.2.0");
    let o = finish(&[]);
    assert!(!o.status.success() && text(&o).contains("trusted comment"), "{}", text(&o));

    // a signature of different bytes
    write_sig(d.path(), &key, &noise(200_000, 8), VERSION);
    let o = finish(&[]);
    assert!(!o.status.success() && text(&o).contains("does not verify"), "{}", text(&o));

    // the exe is not the one the unsigned manifest describes
    write_sig(d.path(), &key, &exe, VERSION);
    let mut changed = exe.clone();
    changed[10] ^= 1;
    std::fs::write(d.path().join("RustyMCLA.exe"), &changed).unwrap();
    let o = finish(&[]);
    assert!(!o.status.success() && text(&o).contains("not the launcher the unsigned manifest describes"), "{}", text(&o));
    std::fs::write(d.path().join("RustyMCLA.exe"), &exe).unwrap();

    // the placeholder public key of a checkout without a signing key
    std::fs::write(&pubkey, "untrusted comment: NOT SET\n").unwrap();
    let o = finish(&[]);
    assert!(!o.status.success() && text(&o).contains("no usable public key"), "{}", text(&o));

    assert!(!d.path().join("manifest.json").exists(), "nothing may be written by a refused run");
}

#[test]
fn finishing_twice_is_refused_for_an_already_signed_manifest() {
    let (d, exe) = draft();
    let key = new_key();
    write_sig(d.path(), &key, &exe, VERSION);
    let pubkey = d.path().join("key.pub");
    std::fs::write(&pubkey, &key.public_text).unwrap();
    let unsigned = d.path().join("manifest.unsigned.json");
    assert!(mk(&["--finish", s(&unsigned), "--assets", s(d.path()), "--pubkey", s(&pubkey)]).status.success());
    let signed = d.path().join("manifest.json");
    let o = mk(&["--finish", s(&signed), "--assets", s(d.path()), "--pubkey", s(&pubkey), "--out", s(&d.path().join("again.json"))]);
    assert!(!o.status.success() && text(&o).contains("already carries a signature"), "{}", text(&o));
}
