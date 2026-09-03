//! The JSON Schema for `skillmgr.yaml`, published so editors and other tools
//! can validate a config without running skillmgr.

use anyhow::Result;
use serde_json::{Value, json};

use crate::config::Config;

/// Where the published schema is served from.
pub const SCHEMA_ID: &str =
    "https://raw.githubusercontent.com/therealm-tech/skillmgr/main/schema/skillmgr.schema.json";

/// Path of the copy committed to this repository.
pub const SCHEMA_PATH: &str = "schema/skillmgr.schema.json";

/// Build the JSON Schema document describing `skillmgr.yaml`.
#[must_use]
pub fn document() -> Value {
    let mut schema = schemars::schema_for!(Config);
    let object = schema.ensure_object();
    object.insert("$id".to_owned(), json!(SCHEMA_ID));
    object.insert(
        "description".to_owned(),
        json!(
            "Declares where Agent Skills come from and which directories they are deployed into."
        ),
    );
    schema.to_value()
}

/// Render the document exactly as the committed copy is stored.
///
/// # Errors
///
/// When the document cannot be serialised, which would mean a schemars bug.
pub fn rendered() -> Result<String> {
    Ok(format!("{}\n", serde_json::to_string_pretty(&document())?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn validator() -> jsonschema::Validator {
        jsonschema::options()
            .should_validate_formats(true)
            .build(&document())
            .expect("the generated document is a valid JSON Schema")
    }

    fn yaml(text: &str) -> Value {
        serde_norway::from_str(text).expect("the fixture is valid YAML")
    }

    #[test]
    fn the_committed_copy_is_up_to_date() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(SCHEMA_PATH);
        let committed = std::fs::read_to_string(&path).expect("the schema is committed");

        assert_eq!(
            committed,
            rendered().unwrap(),
            "{SCHEMA_PATH} is stale; regenerate it with `scripts/generate-schema.sh`"
        );
    }

    #[test]
    fn accepts_the_documented_example() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/skillmgr.yaml");
        let example = yaml(&std::fs::read_to_string(path).unwrap());

        let errors: Vec<String> = validator()
            .iter_errors(&example)
            .map(|error| error.to_string())
            .collect();

        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn accepts_a_minimal_config() {
        let config = yaml("repos:\n  - repo: local\n    paths:\n      - path: skills\n");

        assert!(validator().is_valid(&config));
    }

    #[test]
    fn rejects_a_git_repo_without_a_revision() {
        let config =
            yaml("repos:\n  - repo: https://example.com/x.git\n    paths:\n      - path: .\n");

        assert!(!validator().is_valid(&config));
    }

    #[test]
    fn rejects_a_local_repo_carrying_a_revision() {
        let config =
            yaml("repos:\n  - repo: local\n    revision: 1.0.0\n    paths:\n      - path: .\n");

        assert!(!validator().is_valid(&config));
    }

    #[test]
    fn rejects_the_singular_target_key() {
        let config =
            yaml("target: .claude/skills\nrepos:\n  - repo: local\n    paths:\n      - path: .\n");

        assert!(!validator().is_valid(&config));
    }

    #[test]
    fn rejects_empty_lists() {
        assert!(!validator().is_valid(&yaml("repos: []\n")));
        assert!(!validator().is_valid(&yaml(
            "targets: []\nrepos:\n  - repo: local\n    paths:\n      - path: .\n"
        )));
        assert!(!validator().is_valid(&yaml("repos:\n  - repo: local\n    paths: []\n")));
    }

    #[test]
    fn rejects_a_misspelled_path_key() {
        let config = yaml("repos:\n  - repo: local\n    paths:\n      - pathz: skills\n");

        assert!(!validator().is_valid(&config));
    }

    #[test]
    fn does_not_leak_the_internal_base_directory() {
        let text = rendered().unwrap();

        assert!(!text.contains("base_dir"), "{text}");
    }
}
