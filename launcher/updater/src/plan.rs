//! What a release means for what is installed: launcher update, game update, data rebuild, skipped or not. Pure functions over
//! versions (semver), so every case is a unit test.

use crate::manifest::ReleaseManifest;
use crate::version;
use serde::Serialize;

/// What is on this machine.
#[derive(Debug, Clone, Default)]
pub struct Installed {
    /// The running launcher.
    pub launcher: String,
    /// The game package in `game/` (its `package.json`); `None` = no game files yet.
    pub game_version: Option<String>,
    /// `data_version` of the game package that is installed (what that game expects of the data).
    pub game_data_version: Option<u32>,
    /// `data_version` the built data was stamped with (`manifest.local.json`); `None` = no data built.
    pub built_data_version: Option<u32>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Plan {
    /// The release's version.
    pub version: String,
    /// The launcher in this release is newer than the running one.
    pub launcher_update: bool,
    /// The game in this release is newer than the installed one, or there is none.
    pub game_update: bool,
    /// No game files are installed at all (first run).
    pub first_install: bool,
    /// The running launcher is older than `min_launcher_version`: the launcher has to be updated before the game package.
    pub launcher_first: bool,
    /// Installing this release's game needs the data rebuilt from the disc (its `data_version` differs from the built data).
    pub update_needs_rebuild: bool,
    /// The installed game and the built data do not belong together right now (a game update was applied, the rebuild is pending).
    pub rebuild_pending: bool,
    /// The user skipped exactly this release.
    pub skipped: bool,
    /// Something to offer: an update that is not skipped.
    pub offer: bool,
}

pub fn plan(m: &ReleaseManifest, inst: &Installed, skipped_version: Option<&str>) -> Plan {
    let launcher_update = version::is_newer(&m.launcher.version, &inst.launcher);
    let first_install = inst.game_version.is_none();
    let game_update = first_install || version::is_newer(&m.game.version, inst.game_version.as_deref().unwrap_or(""));
    let launcher_first = version::is_newer(&m.min_launcher_version, &inst.launcher);
    let update_needs_rebuild = game_update && inst.built_data_version.map(|d| d != m.game.data_version).unwrap_or(false);
    let rebuild_pending = match (inst.game_data_version, inst.built_data_version) {
        (Some(g), Some(b)) => g != b,
        _ => false,
    };
    let skipped = skipped_version.and_then(version::normalise).map(|s| s == m.version).unwrap_or(false);
    let anything = launcher_update || (game_update && !first_install);
    Plan { version: m.version.clone(), launcher_update, game_update, first_install, launcher_first, update_needs_rebuild, rebuild_pending, skipped, offer: anything && !skipped }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::tests::sample;

    fn inst(launcher: &str, game: Option<&str>, game_dv: Option<u32>, built_dv: Option<u32>) -> Installed {
        Installed { launcher: launcher.into(), game_version: game.map(Into::into), game_data_version: game_dv, built_data_version: built_dv }
    }

    #[test]
    fn up_to_date_offers_nothing() {
        let m = sample(); // 0.2.0 / 0.2.0, data 1
        let p = plan(&m, &inst("0.2.0", Some("0.2.0"), Some(1), Some(1)), None);
        assert!(!p.launcher_update && !p.game_update && !p.offer && !p.rebuild_pending);
    }

    #[test]
    fn newer_game_and_launcher_are_offered() {
        let m = sample();
        let p = plan(&m, &inst("0.1.0", Some("0.1.0"), Some(1), Some(1)), None);
        assert!(p.launcher_update && p.game_update && p.offer);
        assert!(!p.update_needs_rebuild);
        assert!(!p.launcher_first);
    }

    #[test]
    fn a_new_data_version_needs_a_rebuild() {
        let mut m = sample();
        m.game.data_version = 2;
        let p = plan(&m, &inst("0.2.0", Some("0.1.0"), Some(1), Some(1)), None);
        assert!(p.game_update && p.update_needs_rebuild && p.offer);
    }

    #[test]
    fn skipping_keeps_the_old_pair_and_hides_the_offer() {
        let mut m = sample();
        m.game.data_version = 2;
        let p = plan(&m, &inst("0.1.0", Some("0.1.0"), Some(1), Some(1)), Some("0.2.0"));
        assert!(p.skipped && !p.offer);
        assert!(!p.rebuild_pending, "old game and old data are still a consistent pair");
        // a skip of 0.2.0 does not hide 0.3.0
        let mut m3 = m.clone();
        m3.version = "0.3.0".into();
        assert!(plan(&m3, &inst("0.1.0", Some("0.1.0"), Some(1), Some(1)), Some("v0.2.0")).offer);
    }

    #[test]
    fn rebuild_is_pending_after_a_game_update_without_a_rebuild() {
        let m = sample();
        let p = plan(&m, &inst("0.2.0", Some("0.2.0"), Some(2), Some(1)), None);
        assert!(p.rebuild_pending);
        assert!(!p.offer);
    }

    #[test]
    fn min_launcher_version_forces_the_launcher_first() {
        let mut m = sample();
        m.min_launcher_version = "0.2.0".into();
        let p = plan(&m, &inst("0.1.0", Some("0.1.0"), Some(1), Some(1)), None);
        assert!(p.launcher_first && p.launcher_update);
    }

    #[test]
    fn first_install_is_not_an_update_offer() {
        let m = sample();
        let p = plan(&m, &inst("0.2.0", None, None, None), None);
        assert!(p.first_install && p.game_update);
        assert!(!p.offer, "nothing installed yet: setup, not an update banner");
        assert!(!p.rebuild_pending && !p.update_needs_rebuild);
    }
}
