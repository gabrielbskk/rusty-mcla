//! Checks against the user's real disc image. Ignored by default: set `MCLA_TEST_ISO` to the .iso and run
//! `cargo test --release -p mcla-install --test real_disc -- --ignored`.

use mcla_install::progress::{CheckStatus, Ctx};
use mcla_install::{Disc, Manifest};
use std::path::PathBuf;

#[test]
#[ignore = "needs MCLA_TEST_ISO"]
fn the_supported_disc_validates() {
    let Some(iso) = std::env::var_os("MCLA_TEST_ISO").map(PathBuf::from) else { return };
    let m = Manifest::embedded();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let s2 = seen.clone();
    let ctx = Ctx::new(
        std::sync::Arc::new(move |e: &mcla_install::Event| {
            if let Some(c) = &e.check {
                s2.lock().unwrap().push((c.id.clone(), c.status));
            }
        }),
        mcla_install::Cancel::new(),
    );
    let d = Disc::open(&iso, &m, &ctx).expect("the supported disc validates");
    assert_eq!(d.identity.rpf_entries, m.iso.rpf_entries);
    assert_eq!(d.identity.title_id, m.iso.title_id);
    let seen = seen.lock().unwrap();
    assert!(seen.iter().all(|(_, s)| *s == CheckStatus::Pass), "{seen:?}");
    assert!(seen.len() >= 7);
}
