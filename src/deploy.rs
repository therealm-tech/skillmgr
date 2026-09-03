//! Putting a skill directory in place, and taking it back out.
//!
//! Every install is staged next to its destination and moved in with a
//! rename, so a run that dies halfway never leaves a half-copied skill for an
//! agent to load.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

/// Prefix of the staging directories an install renames from.
pub const STAGING_PREFIX: &str = ".skillmgr-staging-";
/// Prefix of the directories a replacement moves the previous version to.
pub const REPLACED_PREFIX: &str = ".skillmgr-replaced-";

/// Fingerprint of a skill directory: every file's path and content.
///
/// # Errors
///
/// When the directory cannot be walked or one of its files cannot be read.
pub fn checksum(dir: &Path) -> Result<String> {
    let mut files: Vec<PathBuf> = Vec::new();
    for entry in WalkDir::new(dir)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|entry| entry.file_name() != ".git")
    {
        let entry = entry.with_context(|| format!("cannot walk {}", dir.display()))?;
        if entry.file_type().is_file() {
            files.push(entry.into_path());
        }
    }
    files.sort();

    let mut hasher = Sha256::new();
    for file in files {
        let relative = file.strip_prefix(dir).unwrap_or(&file);
        hasher.update(relative.to_string_lossy().as_bytes());
        hasher.update([0]);
        let contents =
            std::fs::read(&file).with_context(|| format!("cannot read {}", file.display()))?;
        hasher.update(contents.len().to_le_bytes());
        hasher.update(&contents);
    }
    Ok(format!("sha256:{}", hex::encode(hasher.finalize())))
}

/// Replace `destination` with a copy of `source`.
///
/// # Errors
///
/// When the copy or either rename fails; the previous version is restored.
pub fn install(source: &Path, destination: &Path) -> Result<()> {
    let parent = destination
        .parent()
        .context("the destination has no parent directory")?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("cannot create {}", parent.display()))?;

    let staging = tempfile::Builder::new()
        .prefix(STAGING_PREFIX)
        .tempdir_in(parent)
        .with_context(|| format!("cannot stage an install in {}", parent.display()))?;
    copy_tree(source, staging.path())?;
    let staged = staging.keep();

    if !destination.exists() {
        return std::fs::rename(&staged, destination).with_context(|| {
            format!(
                "cannot move {} into place at {}",
                staged.display(),
                destination.display()
            )
        });
    }

    let replaced = tempfile::Builder::new()
        .prefix(REPLACED_PREFIX)
        .tempdir_in(parent)?
        .keep();
    std::fs::remove_dir(&replaced)?;
    std::fs::rename(destination, &replaced)
        .with_context(|| format!("cannot move {} aside", destination.display()))?;

    if let Err(error) = std::fs::rename(&staged, destination) {
        std::fs::rename(&replaced, destination).ok();
        return Err(error).with_context(|| format!("cannot install {}", destination.display()));
    }
    std::fs::remove_dir_all(&replaced).ok();
    Ok(())
}

/// Delete an installed skill.
///
/// # Errors
///
/// When the directory exists but cannot be deleted.
pub fn remove(destination: &Path) -> Result<()> {
    if !destination.exists() {
        return Ok(());
    }
    std::fs::remove_dir_all(destination)
        .with_context(|| format!("cannot remove {}", destination.display()))
}

/// Delete the staging leftovers of a run that was killed mid-install.
///
/// # Errors
///
/// When the target directory cannot be listed.
pub fn sweep_staging(target: &Path) -> Result<()> {
    if !target.is_dir() {
        return Ok(());
    }
    for entry in
        std::fs::read_dir(target).with_context(|| format!("cannot read {}", target.display()))?
    {
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if name.starts_with(STAGING_PREFIX) || name.starts_with(REPLACED_PREFIX) {
            tracing::debug!(path = %path.display(), "removing a leftover staging directory");
            std::fs::remove_dir_all(&path).ok();
        }
    }
    Ok(())
}

fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    for entry in WalkDir::new(source)
        .min_depth(1)
        .into_iter()
        .filter_entry(|entry| entry.file_name() != ".git")
    {
        let entry = entry.with_context(|| format!("cannot walk {}", source.display()))?;
        let relative = entry.path().strip_prefix(source)?;
        let target = destination.join(relative);

        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&target)
                .with_context(|| format!("cannot create {}", target.display()))?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(entry.path(), &target)
                .with_context(|| format!("cannot copy {}", entry.path().display()))?;
        } else {
            tracing::warn!(path = %entry.path().display(), "skipping an entry that is neither a file nor a directory");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(root: &Path, files: &[(&str, &str)]) -> PathBuf {
        for (relative, contents) in files {
            let path = root.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, contents).unwrap();
        }
        root.to_path_buf()
    }

    #[test]
    fn checksum_ignores_the_git_directory() {
        let one = tempfile::tempdir().unwrap();
        let two = tempfile::tempdir().unwrap();
        tree(one.path(), &[("SKILL.md", "body")]);
        tree(two.path(), &[("SKILL.md", "body"), (".git/HEAD", "ref: x")]);

        assert_eq!(checksum(one.path()).unwrap(), checksum(two.path()).unwrap());
    }

    #[test]
    fn checksum_tracks_content_and_layout() {
        let root = tempfile::tempdir().unwrap();
        let one = tree(&root.path().join("one"), &[("SKILL.md", "a")]);
        let two = tree(&root.path().join("two"), &[("SKILL.md", "b")]);
        let three = tree(&root.path().join("three"), &[("scripts/SKILL.md", "a")]);

        assert_ne!(checksum(&one).unwrap(), checksum(&two).unwrap());
        assert_ne!(checksum(&one).unwrap(), checksum(&three).unwrap());
    }

    #[test]
    fn installs_a_tree_and_replaces_it() {
        let root = tempfile::tempdir().unwrap();
        let first = tree(
            &root.path().join("v1"),
            &[("SKILL.md", "one"), ("scripts/run.sh", "#!/bin/sh")],
        );
        let second = tree(&root.path().join("v2"), &[("SKILL.md", "two")]);
        let destination = root.path().join("target/demo");

        install(&first, &destination).unwrap();
        assert_eq!(
            std::fs::read_to_string(destination.join("SKILL.md")).unwrap(),
            "one"
        );
        assert!(destination.join("scripts/run.sh").is_file());

        install(&second, &destination).unwrap();
        assert_eq!(
            std::fs::read_to_string(destination.join("SKILL.md")).unwrap(),
            "two"
        );
        assert!(
            !destination.join("scripts/run.sh").exists(),
            "a replacement must not leave the previous version's files behind"
        );
    }

    #[test]
    fn install_leaves_no_staging_directory_behind() {
        let root = tempfile::tempdir().unwrap();
        let source = tree(&root.path().join("v1"), &[("SKILL.md", "one")]);
        let target = root.path().join("target");

        install(&source, &target.join("demo")).unwrap();
        install(&source, &target.join("demo")).unwrap();

        let leftovers: Vec<_> = std::fs::read_dir(&target)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with('.'))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn sweeps_leftovers_from_an_interrupted_run() {
        let target = tempfile::tempdir().unwrap();
        std::fs::create_dir(target.path().join(format!("{STAGING_PREFIX}abc"))).unwrap();
        std::fs::create_dir(target.path().join(format!("{REPLACED_PREFIX}def"))).unwrap();
        std::fs::create_dir(target.path().join("keep-me")).unwrap();

        sweep_staging(target.path()).unwrap();

        let remaining: Vec<_> = std::fs::read_dir(target.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(remaining, ["keep-me"]);
    }

    #[test]
    fn removing_an_absent_skill_is_not_an_error() {
        let root = tempfile::tempdir().unwrap();

        assert!(remove(&root.path().join("absent")).is_ok());
    }
}
