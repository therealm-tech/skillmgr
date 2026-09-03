//! What skillmgr installed, so it knows what it may replace and remove.
//!
//! The file lives in the target directory rather than next to the config: the
//! config is shared and version-controlled, whereas what is on disk is a
//! property of this machine.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Name of the state file inside the target directory.
pub const STATE_FILE: &str = ".skillmgr.json";

const CURRENT_VERSION: u32 = 1;

/// The record of every skill skillmgr owns in one target directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    /// Schema version of this file.
    pub version: u32,
    /// Installed skills, keyed by the name they are deployed under.
    pub skills: BTreeMap<String, InstalledSkill>,
}

/// One deployed skill, and where it came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledSkill {
    /// The `repo:` value it was declared under.
    pub repo: String,
    /// The `revision:` it was pinned to.
    pub revision: Option<String>,
    /// The commit that revision resolved to.
    pub commit: Option<String>,
    /// Where the skill sits inside its source.
    pub source_path: String,
    /// Fingerprint of the installed tree.
    pub checksum: String,
}

impl Default for State {
    fn default() -> Self {
        Self {
            version: CURRENT_VERSION,
            skills: BTreeMap::new(),
        }
    }
}

impl State {
    /// Read the state of `target`, defaulting to empty when there is none.
    ///
    /// # Errors
    ///
    /// When the file cannot be read, is corrupt, or was written by a newer
    /// skillmgr.
    pub fn load(target: &Path) -> Result<Self> {
        let path = Self::path(target);
        if !path.is_file() {
            return Ok(Self::default());
        }

        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("cannot read {}", path.display()))?;
        let state: Self = serde_json::from_str(&text).with_context(|| {
            format!(
                "{} is corrupt; remove it to start from scratch",
                path.display()
            )
        })?;
        anyhow::ensure!(
            state.version == CURRENT_VERSION,
            "{} was written by a newer skillmgr (state version {}); upgrade skillmgr",
            path.display(),
            state.version
        );
        Ok(state)
    }

    /// Write the state of `target`, atomically.
    ///
    /// # Errors
    ///
    /// When the target directory cannot be created or written to.
    pub fn save(&self, target: &Path) -> Result<()> {
        let path = Self::path(target);
        std::fs::create_dir_all(target)
            .with_context(|| format!("cannot create {}", target.display()))?;
        let text = serde_json::to_string_pretty(self)?;

        let mut staging = tempfile::Builder::new()
            .prefix(".skillmgr-state-")
            .tempfile_in(target)
            .with_context(|| format!("cannot write into {}", target.display()))?;
        std::io::Write::write_all(&mut staging, text.as_bytes())?;
        std::io::Write::write_all(&mut staging, b"\n")?;
        staging
            .persist(&path)
            .with_context(|| format!("cannot write {}", path.display()))?;
        Ok(())
    }

    /// Path of the state file inside `target`.
    #[must_use]
    pub fn path(target: &Path) -> PathBuf {
        target.join(STATE_FILE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> InstalledSkill {
        InstalledSkill {
            repo: "https://github.com/toto/tata".to_owned(),
            revision: Some("1.2.3".to_owned()),
            commit: Some("deadbeef".to_owned()),
            source_path: "mydir/demo".to_owned(),
            checksum: "sha256:00".to_owned(),
        }
    }

    #[test]
    fn an_absent_state_file_reads_as_empty() {
        let target = tempfile::tempdir().unwrap();

        assert!(State::load(target.path()).unwrap().skills.is_empty());
    }

    #[test]
    fn round_trips_through_the_target_directory() {
        let target = tempfile::tempdir().unwrap();
        let mut state = State::default();
        state.skills.insert("demo".to_owned(), entry());

        state.save(target.path()).unwrap();
        let reloaded = State::load(target.path()).unwrap();

        assert_eq!(reloaded.skills["demo"], entry());
    }

    #[test]
    fn refuses_a_state_file_from_a_newer_version() {
        let target = tempfile::tempdir().unwrap();
        std::fs::write(State::path(target.path()), r#"{"version":99,"skills":{}}"#).unwrap();

        let error = State::load(target.path()).unwrap_err().to_string();

        assert!(error.contains("newer skillmgr"), "{error}");
    }

    #[test]
    fn refuses_a_corrupt_state_file() {
        let target = tempfile::tempdir().unwrap();
        std::fs::write(State::path(target.path()), "not json").unwrap();

        assert!(State::load(target.path()).is_err());
    }
}
