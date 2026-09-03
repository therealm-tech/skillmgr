//! The whole pipeline against a real git repository and a real target
//! directory: config, fetch, discovery, install, prune.

use std::path::Path;
use std::process::Command;

use skillmgr::command::update::{Action, Report, update};
use skillmgr::config::Config;
use skillmgr::source::Fetcher;
use skillmgr::state::State;

#[tokio::test]
async fn deploys_skills_from_a_git_tag_and_from_the_config_directory() {
    let origin = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();

    write_skill(&origin.path().join("skills/from-git"), "from-git");
    write_skill(&origin.path().join("skills/experimental"), "experimental");
    commit_and_tag(origin.path(), "v1.0.0");

    write_skill(
        &workspace.path().join("local-skills/from-local"),
        "from-local",
    );
    write_skill(&workspace.path().join("local-skills/draft/wip"), "wip");

    let config_path = workspace.path().join("skillmgr.yaml");
    std::fs::write(
        &config_path,
        format!(
            "targets:\n\
             \x20 - deployed\n\
             repos:\n\
             \x20 - repo: {origin}\n\
             \x20   revision: v1.0.0\n\
             \x20   paths:\n\
             \x20     - path: skills\n\
             \x20       exclude: ^experimental$\n\
             \x20 - repo: local\n\
             \x20   paths:\n\
             \x20     - path: local-skills\n\
             \x20       recurse: true\n\
             \x20       exclude: ^draft/\n",
            origin = origin.path().display()
        ),
    )
    .unwrap();

    let config = Config::load(&config_path).unwrap();
    let target = workspace.path().join("deployed");
    let fetcher = Fetcher::new(config.base_dir.clone(), cache.path().to_path_buf(), false);

    let report = update(
        &config,
        std::slice::from_ref(&target),
        &fetcher,
        false,
        false,
    )
    .await
    .unwrap();

    let mut installed: Vec<_> = report.summaries[0]
        .changes
        .iter()
        .map(|change| (change.action, change.name.as_str()))
        .collect();
    installed.sort_by_key(|(_, name)| *name);
    assert_eq!(
        installed,
        [(Action::Added, "from-git"), (Action::Added, "from-local")]
    );
    assert!(target.join("from-git/SKILL.md").is_file());
    assert!(target.join("from-local/SKILL.md").is_file());
    assert!(!target.join("experimental").exists());
    assert!(!target.join("wip").exists());

    let state = State::load(&target).unwrap();
    assert_eq!(state.skills["from-git"].revision.as_deref(), Some("v1.0.0"));
    assert_eq!(state.skills["from-git"].commit.as_ref().unwrap().len(), 40);
    assert_eq!(state.skills["from-local"].repo, "local");

    let offline = Fetcher::new(config.base_dir.clone(), cache.path().to_path_buf(), true);
    let second: Report = update(
        &config,
        std::slice::from_ref(&target),
        &offline,
        false,
        false,
    )
    .await
    .unwrap();
    assert_eq!(second.count(Action::Unchanged), 2);
}

#[tokio::test]
async fn a_moved_tag_reinstalls_the_skill() {
    let origin = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();

    write_skill(&origin.path().join("skills/demo"), "demo");
    commit_and_tag(origin.path(), "v1.0.0");

    let config_path = workspace.path().join("skillmgr.yaml");
    std::fs::write(
        &config_path,
        format!(
            "repos:\n\
             \x20 - repo: {origin}\n\
             \x20   revision: main\n\
             \x20   paths:\n\
             \x20     - path: skills\n",
            origin = origin.path().display()
        ),
    )
    .unwrap();

    let config = Config::load(&config_path).unwrap();
    let target = workspace.path().join("deployed");
    let fetcher = Fetcher::new(config.base_dir.clone(), cache.path().to_path_buf(), false);

    update(
        &config,
        std::slice::from_ref(&target),
        &fetcher,
        false,
        false,
    )
    .await
    .unwrap();

    std::fs::write(
        origin.path().join("skills/demo/SKILL.md"),
        "---\nname: demo\ndescription: Now with more demo.\n---\n",
    )
    .unwrap();
    git(origin.path(), &["add", "."]);
    git(origin.path(), &["commit", "--quiet", "--message", "update"]);

    let report = update(
        &config,
        std::slice::from_ref(&target),
        &fetcher,
        false,
        false,
    )
    .await
    .unwrap();

    assert_eq!(report.count(Action::Updated), 1);
    assert!(
        std::fs::read_to_string(target.join("demo/SKILL.md"))
            .unwrap()
            .contains("Now with more demo.")
    );
}

fn write_skill(dir: &Path, name: &str) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(
        dir.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: The {name} skill, for tests.\n---\n\nBody.\n"),
    )
    .unwrap();
}

fn commit_and_tag(root: &Path, tag: &str) {
    for args in [
        vec!["init", "--quiet", "--initial-branch", "main"],
        vec!["config", "user.email", "test@example.invalid"],
        vec!["config", "user.name", "skillmgr tests"],
        vec!["config", "commit.gpgsign", "false"],
        vec!["config", "tag.gpgsign", "false"],
        vec!["add", "."],
        vec!["commit", "--quiet", "--message", "seed"],
        vec!["tag", tag],
    ] {
        git(root, &args);
    }
}

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Drive `skillmgr validate` exactly as the command line would.
async fn run_validate(args: &[&str]) -> anyhow::Result<()> {
    use clap::Parser as _;

    let cli =
        skillmgr::cli::Cli::parse_from(std::iter::once("skillmgr").chain(args.iter().copied()));
    let skillmgr::cli::Command::Validate {
        configs,
        config_only,
    } = &cli.command
    else {
        panic!("not a validate invocation");
    };
    let (configs, config_only) = (configs.clone(), *config_only);
    skillmgr::command::validate::run(&cli, &configs, config_only).await
}

fn write_config(dir: &Path, name: &str, body: &str) -> String {
    let path = dir.join(name);
    std::fs::write(&path, body).unwrap();
    path.to_str().unwrap().to_owned()
}

#[tokio::test]
async fn config_only_validation_checks_every_file_it_is_given() {
    let workspace = tempfile::tempdir().unwrap();
    let good = write_config(
        workspace.path(),
        "skillmgr.yaml",
        "repos:\n  - repo: local\n    paths:\n      - path: skills\n",
    );
    let bad = write_config(
        workspace.path(),
        "broken.yaml",
        "repos:\n  - repo: https://example.invalid/x.git\n    paths:\n      - path: .\n",
    );

    assert!(
        run_validate(&["validate", "--config-only", &good])
            .await
            .is_ok()
    );

    let error = run_validate(&["validate", "--config-only", &good, &bad])
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("1 of 2"), "{error}");
}

#[tokio::test]
async fn config_only_validation_falls_back_to_the_config_flag() {
    let workspace = tempfile::tempdir().unwrap();
    let path = write_config(
        workspace.path(),
        "skillmgr.yaml",
        "repos:\n  - repo: local\n    paths:\n      - path: skills\n",
    );

    assert!(
        run_validate(&["validate", "--config-only", "--config", &path])
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn config_only_validation_does_not_look_at_the_skills() {
    let workspace = tempfile::tempdir().unwrap();
    let broken = workspace.path().join("skills/Bad-Name");
    std::fs::create_dir_all(&broken).unwrap();
    std::fs::write(
        broken.join("SKILL.md"),
        "---\nname: Bad-Name\ndescription: outside the specification\n---\n",
    )
    .unwrap();
    let path = write_config(
        workspace.path(),
        "skillmgr.yaml",
        "repos:\n  - repo: local\n    paths:\n      - path: skills\n",
    );

    assert!(
        run_validate(&["validate", "--config-only", &path])
            .await
            .is_ok(),
        "the config itself is fine, and --config-only claims nothing more"
    );

    let error = run_validate(&["validate", &path])
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("Agent Skills specification"), "{error}");
}

#[tokio::test]
async fn validation_reports_a_config_that_does_not_exist() {
    let workspace = tempfile::tempdir().unwrap();
    let missing = workspace.path().join("absent.yaml");

    assert!(
        run_validate(&["validate", "--config-only", missing.to_str().unwrap()])
            .await
            .is_err()
    );
}
