//! Turning the config into the list of skills that should be deployed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::config::Config;
use crate::skill;
use crate::source::Materializer;
use crate::{deploy, discovery};

/// One skill the config selects, ready to be installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planned {
    /// The name the skill deploys under.
    pub name: String,
    /// Where its files currently sit.
    pub source_dir: PathBuf,
    /// The `repo:` value that selected it.
    pub repo: String,
    /// The `revision:` that source is pinned to.
    pub revision: Option<String>,
    /// The commit the revision resolved to.
    pub commit: Option<String>,
    /// Its path inside the source.
    pub source_path: String,
    /// Fingerprint of its tree.
    pub checksum: String,
}

impl Planned {
    /// How the skill's origin reads in output.
    #[must_use]
    pub fn origin(&self) -> String {
        match &self.revision {
            Some(revision) => format!("{}@{revision} {}", self.repo, self.source_path),
            None => format!("{} {}", self.repo, self.source_path),
        }
    }
}

/// The outcome of walking every source the config declares.
#[derive(Debug, Default)]
pub struct Plan {
    /// The skills that passed validation, keyed by name and ordered by it.
    pub skills: Vec<Planned>,
    /// Skills that were found but rejected, one message each.
    pub problems: Vec<String>,
}

/// Materialize every source and collect the skills its paths select.
///
/// # Errors
///
/// When a source cannot be fetched, a declared path is missing, or two
/// sources provide the same skill name.
pub async fn build(config: &Config, materializer: &impl Materializer) -> Result<Plan> {
    let mut skills: BTreeMap<String, Planned> = BTreeMap::new();
    let mut problems = Vec::new();

    for repo in &config.repos {
        let label = repo.label();
        let fetched = materializer
            .materialize(repo)
            .await
            .with_context(|| format!("cannot make {label} available"))?;
        let root = fetched
            .root
            .canonicalize()
            .with_context(|| format!("{} does not exist", fetched.root.display()))?;

        for spec in &repo.paths {
            for dir in discovery::discover(&root, spec)? {
                let frontmatter = match skill::load(&dir) {
                    Ok(frontmatter) => frontmatter,
                    Err(error) => {
                        problems.push(format!("{label}: {error}"));
                        continue;
                    }
                };

                let planned = Planned {
                    name: frontmatter.name,
                    source_path: relative_slug(&root, &dir),
                    checksum: deploy::checksum(&dir)?,
                    source_dir: dir,
                    repo: repo.repo.clone(),
                    revision: repo.revision.clone(),
                    commit: fetched.resolved.clone(),
                };

                if let Some(existing) = skills.get(&planned.name) {
                    bail!(
                        "two sources provide the skill `{}`: {} and {}",
                        planned.name,
                        existing.origin(),
                        planned.origin()
                    );
                }
                tracing::debug!(skill = %planned.name, origin = %planned.origin(), "selected");
                skills.insert(planned.name.clone(), planned);
            }
        }
    }

    Ok(Plan {
        skills: skills.into_values().collect(),
        problems,
    })
}

fn relative_slug(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}
