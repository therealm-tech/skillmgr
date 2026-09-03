//! The unit skillmgr deploys: a directory holding a `SKILL.md` whose YAML
//! frontmatter follows the Agent Skills specification.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;
use validator::{Validate, ValidationError};

/// The file that makes a directory a skill.
pub const SKILL_FILE: &str = "SKILL.md";

/// Everything that can make a directory fail to be a usable skill.
#[derive(Debug, Error)]
pub enum SkillError {
    /// The directory carries no `SKILL.md`.
    #[error("{path}: no {SKILL_FILE}")]
    NotASkill {
        /// The directory that was inspected.
        path: PathBuf,
    },
    /// The `SKILL.md` does not open with a YAML frontmatter block.
    #[error("{path}: {SKILL_FILE} has no YAML frontmatter (it must open with a `---` line)")]
    NoFrontmatter {
        /// The offending skill directory.
        path: PathBuf,
    },
    /// The frontmatter block is not parseable YAML.
    #[error("{path}: {SKILL_FILE} frontmatter is not valid YAML: {source}")]
    Yaml {
        /// The offending skill directory.
        path: PathBuf,
        /// The parser's complaint.
        source: serde_norway::Error,
    },
    /// The frontmatter parses but breaks the specification.
    #[error("{path}: {SKILL_FILE} frontmatter is invalid: {source}")]
    Invalid {
        /// The offending skill directory.
        path: PathBuf,
        /// The failing constraints.
        source: validator::ValidationErrors,
    },
    /// The `name` field disagrees with the directory it lives in.
    #[error("{path}: frontmatter name `{name}` does not match the directory name")]
    NameMismatch {
        /// The offending skill directory.
        path: PathBuf,
        /// The name the frontmatter claims.
        name: String,
    },
    /// The `SKILL.md` could not be read.
    #[error("{path}: cannot read {SKILL_FILE}: {source}")]
    Io {
        /// The offending skill directory.
        path: PathBuf,
        /// The underlying failure.
        source: std::io::Error,
    },
}

/// The frontmatter fields defined by the Agent Skills specification.
///
/// Unknown keys are kept out of this struct on purpose: agents extend the
/// frontmatter with their own fields (`model`, `argument-hint`, `paths`, …)
/// and rejecting those would make skillmgr refuse perfectly good skills.
#[derive(Debug, Clone, Deserialize, Validate)]
pub struct Frontmatter {
    /// Skill identifier, and the directory name it must be stored under.
    #[validate(length(min = 1, max = 64), custom(function = validate_name))]
    pub name: String,
    /// What the skill does and when an agent should reach for it.
    #[validate(length(min = 1, max = 1024))]
    pub description: String,
    /// Environment requirements, when the skill has any.
    #[validate(length(min = 1, max = 500))]
    pub compatibility: Option<String>,
    /// Client-defined extra properties.
    #[serde(default)]
    pub metadata: BTreeMap<String, serde_norway::Value>,
}

/// Read and validate the skill stored in `dir`.
///
/// # Errors
///
/// When the directory holds no `SKILL.md`, or its frontmatter is missing,
/// unparseable, or outside the Agent Skills specification.
pub fn load(dir: &Path) -> Result<Frontmatter, SkillError> {
    let manifest = dir.join(SKILL_FILE);
    if !manifest.is_file() {
        return Err(SkillError::NotASkill {
            path: dir.to_path_buf(),
        });
    }

    let text = std::fs::read_to_string(&manifest).map_err(|source| SkillError::Io {
        path: dir.to_path_buf(),
        source,
    })?;
    let block = split_frontmatter(&text).ok_or_else(|| SkillError::NoFrontmatter {
        path: dir.to_path_buf(),
    })?;
    let frontmatter: Frontmatter =
        serde_norway::from_str(block).map_err(|source| SkillError::Yaml {
            path: dir.to_path_buf(),
            source,
        })?;
    frontmatter
        .validate()
        .map_err(|source| SkillError::Invalid {
            path: dir.to_path_buf(),
            source,
        })?;

    let dir_name = dir.file_name().and_then(|name| name.to_str());
    if dir_name != Some(frontmatter.name.as_str()) {
        return Err(SkillError::NameMismatch {
            path: dir.to_path_buf(),
            name: frontmatter.name,
        });
    }

    for (key, value) in &frontmatter.metadata {
        if !value.is_string() {
            tracing::warn!(
                skill = %frontmatter.name,
                %key,
                "metadata value is not a string; the specification maps string keys to string values"
            );
        }
    }

    Ok(frontmatter)
}

/// Return the YAML between the opening `---` line and the closing one.
fn split_frontmatter(text: &str) -> Option<&str> {
    let body = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))?;
    let mut offset = 0;
    for line in body.split_inclusive('\n') {
        if line.trim_end() == "---" {
            return Some(&body[..offset]);
        }
        offset += line.len();
    }
    None
}

fn validate_name(name: &str) -> Result<(), ValidationError> {
    let error = |message: &'static str| {
        let mut failure = ValidationError::new("skill_name");
        failure.message = Some(message.into());
        failure
    };

    if !name.chars().all(|character| {
        character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
    }) {
        return Err(error(
            "must contain only lowercase letters, digits and hyphens",
        ));
    }
    if name.starts_with('-') || name.ends_with('-') {
        return Err(error("must not start or end with a hyphen"));
    }
    if name.contains("--") {
        return Err(error("must not contain consecutive hyphens"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_skill(dir: &Path, name: &str, body: &str) -> PathBuf {
        let skill_dir = dir.join(name);
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join(SKILL_FILE), body).unwrap();
        skill_dir
    }

    #[test]
    fn loads_a_minimal_skill() {
        let root = tempfile::tempdir().unwrap();
        let dir = write_skill(
            root.path(),
            "pdf-processing",
            "---\nname: pdf-processing\ndescription: Extract text from PDFs.\n---\n\nBody.\n",
        );

        let frontmatter = load(&dir).unwrap();

        assert_eq!(frontmatter.name, "pdf-processing");
        assert_eq!(frontmatter.description, "Extract text from PDFs.");
    }

    #[test]
    fn keeps_agent_specific_frontmatter_extensions() {
        let root = tempfile::tempdir().unwrap();
        let dir = write_skill(
            root.path(),
            "deploy",
            "---\nname: deploy\ndescription: Ship it.\nmodel: opus\ndisable-model-invocation: true\n---\n",
        );

        assert!(load(&dir).is_ok());
    }

    #[test]
    fn rejects_a_name_that_disagrees_with_the_directory() {
        let root = tempfile::tempdir().unwrap();
        let dir = write_skill(
            root.path(),
            "deploy",
            "---\nname: ship\ndescription: Ship it.\n---\n",
        );

        assert!(matches!(load(&dir), Err(SkillError::NameMismatch { .. })));
    }

    #[test]
    fn rejects_a_name_outside_the_specification() {
        let root = tempfile::tempdir().unwrap();
        let dir = write_skill(
            root.path(),
            "PDF--Processing",
            "---\nname: PDF--Processing\ndescription: Nope.\n---\n",
        );

        assert!(matches!(load(&dir), Err(SkillError::Invalid { .. })));
    }

    #[test]
    fn rejects_an_empty_description() {
        let root = tempfile::tempdir().unwrap();
        let dir = write_skill(
            root.path(),
            "deploy",
            "---\nname: deploy\ndescription: \"\"\n---\n",
        );

        assert!(matches!(load(&dir), Err(SkillError::Invalid { .. })));
    }

    #[test]
    fn rejects_a_directory_without_frontmatter() {
        let root = tempfile::tempdir().unwrap();
        let dir = write_skill(root.path(), "deploy", "# Just markdown\n");

        assert!(matches!(load(&dir), Err(SkillError::NoFrontmatter { .. })));
    }

    #[test]
    fn rejects_a_directory_without_a_skill_file() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("empty")).unwrap();

        assert!(matches!(
            load(&root.path().join("empty")),
            Err(SkillError::NotASkill { .. })
        ));
    }

    #[test]
    fn splits_frontmatter_at_the_closing_marker() {
        assert_eq!(split_frontmatter("---\na: 1\n---\nbody\n"), Some("a: 1\n"));
        assert_eq!(split_frontmatter("no marker\n"), None);
        assert_eq!(split_frontmatter("---\nunterminated\n"), None);
    }
}
