//! Finding skill directories inside a materialized source.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use walkdir::WalkDir;

use crate::config::PathSpec;
use crate::skill::SKILL_FILE;
use crate::source::contained_join;

/// List the skill directories a `paths:` entry selects, in a stable order.
///
/// # Errors
///
/// When the entry's path is missing, is not a directory, escapes the source
/// root, or cannot be walked.
pub fn discover(root: &Path, spec: &PathSpec) -> Result<Vec<PathBuf>> {
    let base = contained_join(root, &spec.path).with_context(|| {
        format!(
            "`{}` does not exist in the source rooted at {}",
            spec.path.display(),
            root.display()
        )
    })?;
    anyhow::ensure!(base.is_dir(), "{} is not a directory", base.display());

    let candidates = if base.join(SKILL_FILE).is_file() {
        vec![base.clone()]
    } else if spec.recurse {
        outermost(&collect_recursive(&base)?)
    } else {
        collect_children(&base)?
    };

    let exclude = spec.exclude_regex()?;
    let mut kept = Vec::new();
    for candidate in candidates {
        let relative = relative_slug(&base, &candidate);
        if exclude
            .as_ref()
            .is_some_and(|pattern| pattern.is_match(&relative))
        {
            tracing::debug!(path = %candidate.display(), %relative, "excluded");
            continue;
        }
        kept.push(candidate);
    }
    kept.sort();
    Ok(kept)
}

fn collect_recursive(base: &Path) -> Result<Vec<PathBuf>> {
    let mut hits = Vec::new();
    let walker = WalkDir::new(base)
        .min_depth(1)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|entry| entry.file_name() != ".git");

    for entry in walker {
        let entry = entry.with_context(|| format!("cannot walk {}", base.display()))?;
        if entry.file_type().is_dir() && entry.path().join(SKILL_FILE).is_file() {
            hits.push(entry.into_path());
        }
    }
    Ok(hits)
}

fn collect_children(base: &Path) -> Result<Vec<PathBuf>> {
    let mut hits = Vec::new();
    let entries =
        std::fs::read_dir(base).with_context(|| format!("cannot read {}", base.display()))?;
    for entry in entries {
        let path = entry
            .with_context(|| format!("cannot read {}", base.display()))?
            .path();
        if path.is_dir() && path.join(SKILL_FILE).is_file() {
            hits.push(path);
        }
    }
    Ok(hits)
}

/// Drop skills nested inside another skill: the outer one already ships them.
fn outermost(hits: &[PathBuf]) -> Vec<PathBuf> {
    hits.iter()
        .filter(|hit| {
            !hits
                .iter()
                .any(|other| other != *hit && hit.starts_with(other))
        })
        .cloned()
        .collect()
}

fn relative_slug(base: &Path, path: &Path) -> String {
    path.strip_prefix(base)
        .unwrap_or(path)
        .components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(path: &str, recurse: bool, exclude: Option<&str>) -> PathSpec {
        PathSpec {
            path: PathBuf::from(path),
            recurse,
            exclude: exclude.map(ToOwned::to_owned),
        }
    }

    fn skill_at(root: &Path, relative: &str) {
        let dir = root.join(relative);
        std::fs::create_dir_all(&dir).unwrap();
        let name = dir.file_name().unwrap().to_str().unwrap();
        std::fs::write(
            dir.join(SKILL_FILE),
            format!("---\nname: {name}\ndescription: x\n---\n"),
        )
        .unwrap();
    }

    fn names(paths: &[PathBuf]) -> Vec<String> {
        paths
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn lists_the_immediate_children_by_default() {
        let root = tempfile::tempdir().unwrap();
        skill_at(root.path(), "mydir/alpha");
        skill_at(root.path(), "mydir/beta");
        skill_at(root.path(), "mydir/nested/gamma");

        let found = discover(root.path(), &spec("mydir", false, None)).unwrap();

        assert_eq!(names(&found), ["alpha", "beta"]);
    }

    #[test]
    fn walks_the_whole_subtree_when_recursing() {
        let root = tempfile::tempdir().unwrap();
        skill_at(root.path(), "mydir/alpha");
        skill_at(root.path(), "mydir/plugins/one/skills/gamma");

        let found = discover(root.path(), &spec("mydir", true, None)).unwrap();

        assert_eq!(names(&found), ["alpha", "gamma"]);
    }

    #[test]
    fn treats_the_path_itself_as_a_skill_when_it_holds_one() {
        let root = tempfile::tempdir().unwrap();
        skill_at(root.path(), "mydir/alpha");

        let found = discover(root.path(), &spec("mydir/alpha", false, None)).unwrap();

        assert_eq!(names(&found), ["alpha"]);
    }

    #[test]
    fn never_descends_into_a_skill_it_already_found() {
        let root = tempfile::tempdir().unwrap();
        skill_at(root.path(), "mydir/alpha");
        skill_at(root.path(), "mydir/alpha/references/beta");

        let found = discover(root.path(), &spec("mydir", true, None)).unwrap();

        assert_eq!(names(&found), ["alpha"]);
    }

    #[test]
    fn applies_exclude_to_the_path_relative_to_the_entry() {
        let root = tempfile::tempdir().unwrap();
        skill_at(root.path(), "mydir/toto-one");
        skill_at(root.path(), "mydir/keep-me");

        let found = discover(root.path(), &spec("mydir", false, Some("^toto"))).unwrap();

        assert_eq!(names(&found), ["keep-me"]);
    }

    #[test]
    fn excludes_by_nested_path_when_recursing() {
        let root = tempfile::tempdir().unwrap();
        skill_at(root.path(), "mydir/draft/alpha");
        skill_at(root.path(), "mydir/stable/beta");

        let found = discover(root.path(), &spec("mydir", true, Some("^draft/"))).unwrap();

        assert_eq!(names(&found), ["beta"]);
    }

    #[test]
    fn skips_the_git_directory() {
        let root = tempfile::tempdir().unwrap();
        skill_at(root.path(), ".git/modules/alpha");
        skill_at(root.path(), "beta");

        let found = discover(root.path(), &spec(".", true, None)).unwrap();

        assert_eq!(names(&found), ["beta"]);
    }

    #[test]
    fn reports_a_missing_path() {
        let root = tempfile::tempdir().unwrap();

        let error = discover(root.path(), &spec("absent", false, None))
            .unwrap_err()
            .to_string();

        assert!(error.contains("absent"), "{error}");
    }
}
