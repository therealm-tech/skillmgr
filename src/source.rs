//! Turning a `repos:` entry into a directory on disk that skills can be read from.

pub mod git;

use std::future::Future;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::config::{RepoSpec, Source};

/// A source made available on the local filesystem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Materialized {
    /// Directory the source's files are rooted at.
    pub root: PathBuf,
    /// The commit a git revision resolved to.
    pub resolved: Option<String>,
}

/// Makes a `repos:` entry's files available locally.
#[cfg_attr(test, mockall::automock)]
pub trait Materializer {
    /// Fetch or locate `repo`'s files and return where they live.
    fn materialize(&self, repo: &RepoSpec) -> impl Future<Output = Result<Materialized>> + Send;
}

/// The production [`Materializer`]: a git cache plus the config's own directory.
#[derive(Debug, Clone)]
pub struct Fetcher {
    base_dir: PathBuf,
    cache_dir: PathBuf,
    offline: bool,
}

impl Fetcher {
    /// Build a fetcher rooted at the config file's directory.
    #[must_use]
    pub fn new(base_dir: PathBuf, cache_dir: PathBuf, offline: bool) -> Self {
        Self {
            base_dir,
            cache_dir,
            offline,
        }
    }
}

impl Materializer for Fetcher {
    fn materialize(&self, repo: &RepoSpec) -> impl Future<Output = Result<Materialized>> + Send {
        let source = repo.source();
        let base_dir = self.base_dir.clone();
        let cache_dir = self.cache_dir.clone();
        let offline = self.offline;

        async move {
            match source {
                Source::Local => Ok(Materialized {
                    root: base_dir,
                    resolved: None,
                }),
                Source::Git { url, revision } => {
                    git::checkout(&url, &revision, &cache_dir, offline).await
                }
            }
        }
    }
}

/// The default cache root, honouring `XDG_CACHE_HOME` through [`dirs`].
#[must_use]
pub fn default_cache_dir() -> PathBuf {
    dirs::cache_dir().map_or_else(
        || PathBuf::from(".skillmgr-cache"),
        |cache| cache.join("skillmgr"),
    )
}

/// Resolve `relative` under `root`, refusing anything that escapes it.
///
/// # Errors
///
/// When either path cannot be canonicalised, or the result leaves `root`.
pub fn contained_join(root: &Path, relative: &Path) -> Result<PathBuf> {
    let joined = root.join(relative);
    let canonical_root = root.canonicalize()?;
    let canonical = joined.canonicalize()?;
    anyhow::ensure!(
        canonical.starts_with(&canonical_root),
        "{} escapes the source root {}",
        relative.display(),
        root.display()
    );
    Ok(canonical)
}
