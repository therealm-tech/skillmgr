//! The declarative input: `skillmgr.yaml`, its schema, and its validation.

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use regex::Regex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use validator::{Validate, ValidationError};

/// Where skills land when neither the CLI nor the config says otherwise.
///
/// Claude Code reads the first; the second is the cross-client convention
/// every other Agent Skills client scans, so one deployment serves both.
pub const DEFAULT_TARGETS: [&str; 2] = [".claude/skills", ".agents/skills"];

/// The sentinel `repo:` value meaning "paths are relative to this config file".
pub const LOCAL_REPO: &str = "local";

/// A parsed and validated `skillmgr.yaml`.
#[derive(Debug, Clone, Deserialize, Serialize, Validate, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "skillmgr configuration")]
pub struct Config {
    /// Directories the skills are deployed into, each getting a full copy.
    ///
    /// A relative path resolves against the working directory, and a leading
    /// `~` is expanded. Defaults to `.claude/skills` and `.agents/skills`.
    #[validate(length(min = 1, message = "at least one target is required"))]
    #[schemars(length(min = 1))]
    pub targets: Option<Vec<PathBuf>>,
    /// The sources to pull skills from.
    #[validate(length(min = 1, message = "at least one repo is required"), nested)]
    #[schemars(length(min = 1))]
    pub repos: Vec<RepoSpec>,
    /// Directory holding the config file; local paths resolve against it.
    #[serde(skip)]
    pub base_dir: PathBuf,
}

/// One source of skills: a git repository, or the config file's own directory.
#[derive(Debug, Clone, Deserialize, Serialize, Validate, JsonSchema)]
#[serde(deny_unknown_fields)]
#[validate(schema(function = check_revision_matches_repo, skip_on_field_errors = true))]
#[schemars(extend(
    "if" = serde_json::json!({"properties": {"repo": {"const": LOCAL_REPO}}, "required": ["repo"]}),
    "then" = serde_json::json!({"properties": {"revision": false}}),
    "else" = serde_json::json!({
        "required": ["revision"],
        "properties": {"revision": {"type": "string", "minLength": 1}}
    }),
))]
pub struct RepoSpec {
    /// A git URL, a path to a git repository, or `local` for the directory
    /// holding this config file.
    #[validate(length(min = 1, message = "must not be empty"))]
    #[schemars(length(min = 1))]
    pub repo: String,
    /// The git revision to check out: a tag, a branch or a commit.
    ///
    /// Required for a git source, and forbidden on `local`.
    pub revision: Option<String>,
    /// Where to look for skills inside the source.
    #[validate(length(min = 1, message = "at least one path is required"), nested)]
    #[schemars(length(min = 1))]
    pub paths: Vec<PathSpec>,
}

/// One place to look for skills inside a source.
#[derive(Debug, Clone, Deserialize, Serialize, Validate, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PathSpec {
    /// Directory to search, relative to the source root.
    ///
    /// A directory holding a `SKILL.md` is itself the skill; otherwise its
    /// subdirectories are searched.
    #[validate(custom(function = check_contained_path))]
    pub path: PathBuf,
    /// Search the whole subtree rather than only the immediate children.
    #[serde(default)]
    pub recurse: bool,
    /// Regular expression rejecting skills whose path under `path` matches.
    #[validate(custom(function = check_regex))]
    #[schemars(extend("format" = "regex"))]
    pub exclude: Option<String>,
}

/// Where a [`RepoSpec`]'s files come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// The directory holding `skillmgr.yaml`.
    Local,
    /// A git repository, pinned to a revision.
    Git {
        /// The URL or path passed to `git`.
        url: String,
        /// The tag, branch or commit to check out.
        revision: String,
    },
}

impl Config {
    /// Read, parse and validate the config file at `path`.
    ///
    /// # Errors
    ///
    /// When the file cannot be read, is not valid YAML, or breaks a rule.
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read the config file {}", path.display()))?;
        let mut config: Self = serde_norway::from_str(&text)
            .with_context(|| format!("{} is not a valid skillmgr config", path.display()))?;
        // `validator` files a struct-level failure under the field name
        // `__all__`, which means nothing to whoever is reading the error.
        config.validate().map_err(|errors| {
            anyhow!(
                "{} is invalid: {}",
                path.display(),
                errors.to_string().replace(".__all__", "")
            )
        })?;
        config.base_dir = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
            .to_path_buf();
        Ok(config)
    }

    /// The directories to deploy into, CLI overrides taking precedence.
    ///
    /// Duplicates are dropped so a directory named twice is written once.
    #[must_use]
    pub fn targets(&self, overrides: &[PathBuf]) -> Vec<PathBuf> {
        let chosen = if overrides.is_empty() {
            self.targets
                .clone()
                .unwrap_or_else(|| DEFAULT_TARGETS.iter().map(PathBuf::from).collect())
        } else {
            overrides.to_vec()
        };

        let mut seen = BTreeSet::new();
        chosen
            .iter()
            .map(|target| expand_home(target))
            .filter(|target| seen.insert(target.clone()))
            .collect()
    }
}

impl RepoSpec {
    /// Where this entry's files come from.
    #[must_use]
    pub fn source(&self) -> Source {
        if self.repo == LOCAL_REPO {
            Source::Local
        } else {
            Source::Git {
                url: self.repo.clone(),
                revision: self.revision.clone().unwrap_or_default(),
            }
        }
    }

    /// How this entry is named in output and in the state file.
    #[must_use]
    pub fn label(&self) -> String {
        match self.source() {
            Source::Local => LOCAL_REPO.to_owned(),
            Source::Git { url, revision } => format!("{url}@{revision}"),
        }
    }
}

impl PathSpec {
    /// The compiled `exclude` pattern.
    ///
    /// Validation already proved the expression compiles, so this only fails
    /// on a `PathSpec` built outside [`Config::load`].
    ///
    /// # Errors
    ///
    /// When the pattern is not a valid regular expression.
    pub fn exclude_regex(&self) -> Result<Option<Regex>> {
        self.exclude
            .as_deref()
            .map(|pattern| {
                Regex::new(pattern)
                    .with_context(|| format!("`{pattern}` is not a valid regular expression"))
            })
            .transpose()
    }
}

/// Replace a leading `~` with the user's home directory.
#[must_use]
pub fn expand_home(path: &Path) -> PathBuf {
    let Ok(rest) = path.strip_prefix("~") else {
        return path.to_path_buf();
    };
    dirs::home_dir().map_or_else(|| path.to_path_buf(), |home| home.join(rest))
}

fn check_revision_matches_repo(repo: &RepoSpec) -> Result<(), ValidationError> {
    let is_local = repo.repo == LOCAL_REPO;
    let has_revision = repo.revision.as_ref().is_some_and(|rev| !rev.is_empty());

    if is_local && has_revision {
        return Err(named_error(
            "revision",
            "`repo: local` reads the working tree, so it takes no revision",
        ));
    }
    if !is_local && !has_revision {
        return Err(named_error(
            "revision",
            "a git repo must be pinned with a revision (a tag, a branch or a commit)",
        ));
    }
    Ok(())
}

fn check_contained_path(path: &Path) -> Result<(), ValidationError> {
    if path.is_absolute() {
        return Err(named_error("path", "must be relative to the source root"));
    }
    if path.components().any(|part| part == Component::ParentDir) {
        return Err(named_error("path", "must not climb out of the source root"));
    }
    Ok(())
}

fn check_regex(pattern: &str) -> Result<(), ValidationError> {
    Regex::new(pattern)
        .map(|_| ())
        .map_err(|_| named_error("exclude", "is not a valid regular expression"))
}

fn named_error(code: &'static str, message: &'static str) -> ValidationError {
    let mut error = ValidationError::new(code);
    error.message = Some(message.into());
    error
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(yaml: &str) -> Result<Config> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("skillmgr.yaml");
        std::fs::write(&path, yaml).unwrap();
        Config::load(&path)
    }

    #[test]
    fn parses_a_git_repo_with_paths() {
        let config = parse(
            "repos:\n  - repo: https://github.com/toto/tata\n    revision: 1.2.3\n    paths:\n      - path: mydir/\n        recurse: true\n        exclude: ^toto\n",
        )
        .unwrap();

        let repo = &config.repos[0];
        assert_eq!(
            repo.source(),
            Source::Git {
                url: "https://github.com/toto/tata".to_owned(),
                revision: "1.2.3".to_owned(),
            }
        );
        assert!(repo.paths[0].recurse);
        assert!(
            repo.paths[0]
                .exclude_regex()
                .unwrap()
                .unwrap()
                .is_match("toto-skill")
        );
    }

    #[test]
    fn defaults_recurse_to_false_and_exclude_to_none() {
        let config = parse("repos:\n  - repo: local\n    paths:\n      - path: skills\n").unwrap();

        assert!(!config.repos[0].paths[0].recurse);
        assert!(config.repos[0].paths[0].exclude.is_none());
    }

    #[test]
    fn rejects_a_git_repo_without_a_revision() {
        let error =
            parse("repos:\n  - repo: https://example.com/x.git\n    paths:\n      - path: .\n")
                .unwrap_err()
                .to_string();

        assert!(error.contains("invalid"), "{error}");
    }

    #[test]
    fn rejects_a_local_repo_carrying_a_revision() {
        assert!(
            parse("repos:\n  - repo: local\n    revision: 1.0.0\n    paths:\n      - path: .\n")
                .is_err()
        );
    }

    #[test]
    fn rejects_an_uncompilable_exclude() {
        assert!(
            parse("repos:\n  - repo: local\n    paths:\n      - path: .\n        exclude: \"[\"\n")
                .is_err()
        );
    }

    #[test]
    fn rejects_a_path_escaping_the_source_root() {
        assert!(
            parse("repos:\n  - repo: local\n    paths:\n      - path: ../elsewhere\n").is_err()
        );
        assert!(parse("repos:\n  - repo: local\n    paths:\n      - path: /etc\n").is_err());
    }

    #[test]
    fn rejects_an_empty_repo_list() {
        assert!(parse("repos: []\n").is_err());
    }

    #[test]
    fn rejects_unknown_keys() {
        assert!(parse("repos:\n  - repo: local\n    pathz:\n      - path: .\n").is_err());
    }

    #[test]
    fn targets_default_to_claude_code_and_the_cross_client_convention() {
        let config = parse("repos:\n  - repo: local\n    paths:\n      - path: .\n").unwrap();

        assert_eq!(
            config.targets(&[]),
            [
                PathBuf::from(".claude/skills"),
                PathBuf::from(".agents/skills")
            ]
        );
    }

    #[test]
    fn targets_prefer_the_cli_overrides_then_the_config() {
        let config = parse(
            "targets:\n  - /opt/skills\nrepos:\n  - repo: local\n    paths:\n      - path: .\n",
        )
        .unwrap();

        assert_eq!(
            config.targets(&[PathBuf::from("/tmp/override")]),
            [PathBuf::from("/tmp/override")]
        );
        assert_eq!(config.targets(&[]), [PathBuf::from("/opt/skills")]);
    }

    #[test]
    fn targets_are_deduplicated() {
        let config = parse("repos:\n  - repo: local\n    paths:\n      - path: .\n").unwrap();
        let repeated = vec![PathBuf::from("skills"), PathBuf::from("skills")];

        assert_eq!(config.targets(&repeated), [PathBuf::from("skills")]);
    }

    #[test]
    fn rejects_an_empty_target_list() {
        assert!(
            parse("targets: []\nrepos:\n  - repo: local\n    paths:\n      - path: .\n").is_err()
        );
    }

    #[test]
    fn expands_a_leading_tilde() {
        let home = dirs::home_dir().unwrap();
        assert_eq!(
            expand_home(Path::new("~/.claude/skills")),
            home.join(".claude/skills")
        );
        assert_eq!(
            expand_home(Path::new("relative/path")),
            PathBuf::from("relative/path")
        );
    }
}
