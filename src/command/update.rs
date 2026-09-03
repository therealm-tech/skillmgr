//! `skillmgr update`: install, refresh and prune the declared skills.

use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};

use crate::cli::Cli;
use crate::command::Context;
use crate::command::plan::{self, Planned};
use crate::config::Config;
use crate::deploy;
use crate::source::Materializer;
use crate::state::{InstalledSkill, State};

/// What happened to one skill during an update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Newly installed.
    Added,
    /// Reinstalled because its source moved on.
    Updated,
    /// Already installed at this exact content.
    Unchanged,
    /// Dropped because the config no longer declares it.
    Removed,
}

impl Action {
    fn symbol(self) -> char {
        match self {
            Self::Added => '+',
            Self::Updated => '~',
            Self::Unchanged => '=',
            Self::Removed => '-',
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Updated => "updated",
            Self::Unchanged => "unchanged",
            Self::Removed => "removed",
        }
    }
}

/// One line of an update's result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// What happened.
    pub action: Action,
    /// The skill it happened to.
    pub name: String,
    /// Where the skill comes from, empty for a removal.
    pub origin: String,
}

/// The result of an update, as printed to stdout.
#[derive(Debug)]
pub struct Summary {
    /// The directory that was updated.
    pub target: PathBuf,
    /// One entry per skill, ordered by name within each action.
    pub changes: Vec<Change>,
    /// Whether the target directory was left untouched.
    pub dry_run: bool,
}

impl Summary {
    /// How many skills the update put in `action`.
    #[must_use]
    pub fn count(&self, action: Action) -> usize {
        self.changes
            .iter()
            .filter(|change| change.action == action)
            .count()
    }
}

impl fmt::Display for Summary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let width = self
            .changes
            .iter()
            .map(|change| change.name.len())
            .max()
            .unwrap_or(0);

        for change in &self.changes {
            let line = format!("{} {:width$}", change.action.symbol(), change.name);
            if change.origin.is_empty() {
                writeln!(formatter, "{}", line.trim_end())?;
            } else {
                writeln!(formatter, "{line}  {}", change.origin)?;
            }
        }

        let counts = [
            Action::Added,
            Action::Updated,
            Action::Unchanged,
            Action::Removed,
        ]
        .into_iter()
        .filter_map(|action| match self.count(action) {
            0 => None,
            count => Some(format!("{count} {}", action.label())),
        })
        .collect::<Vec<_>>()
        .join(", ");

        let installed = self.changes.len() - self.count(Action::Removed);
        let suffix = if self.dry_run { " (dry run)" } else { "" };
        let detail = if counts.is_empty() {
            String::new()
        } else {
            format!(": {counts}")
        };
        let noun = if installed == 1 { "skill" } else { "skills" };
        writeln!(
            formatter,
            "{installed} {noun} in {}{detail}{suffix}",
            self.target.display()
        )
    }
}

/// What one update did, across every target directory.
#[derive(Debug)]
pub struct Report {
    /// One summary per target, in the order the targets were resolved.
    pub summaries: Vec<Summary>,
}

impl Report {
    /// How many skills across every target the update put in `action`.
    #[must_use]
    pub fn count(&self, action: Action) -> usize {
        self.summaries
            .iter()
            .map(|summary| summary.count(action))
            .sum()
    }
}

impl fmt::Display for Report {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, summary) in self.summaries.iter().enumerate() {
            if index > 0 {
                writeln!(formatter)?;
            }
            write!(formatter, "{summary}")?;
        }
        Ok(())
    }
}

/// Run the subcommand.
///
/// # Errors
///
/// When the config cannot be loaded, or the update itself fails.
pub async fn run(cli: &Cli, dry_run: bool, force: bool) -> Result<()> {
    let context = Context::open(cli)?;
    let fetcher = context.fetcher(cli);
    let report = update(&context.config, &context.targets, &fetcher, dry_run, force).await?;
    print!("{report}");
    Ok(())
}

/// Bring every directory in `targets` in line with `config`.
///
/// The sources are fetched and validated once, then applied to each target.
///
/// # Errors
///
/// When a source cannot be fetched, a selected skill is invalid, two sources
/// provide the same name, or a target holds an unmanaged directory of that
/// name and `force` is not set.
pub async fn update(
    config: &Config,
    targets: &[PathBuf],
    materializer: &impl Materializer,
    dry_run: bool,
    force: bool,
) -> Result<Report> {
    let plan = plan::build(config, materializer).await?;
    if !plan.problems.is_empty() {
        for problem in &plan.problems {
            tracing::error!("{problem}");
        }
        bail!(
            "{} skill(s) do not satisfy the Agent Skills specification",
            plan.problems.len()
        );
    }

    let mut summaries = Vec::with_capacity(targets.len());
    for target in targets {
        summaries.push(apply(&plan.skills, target, dry_run, force)?);
    }
    Ok(Report { summaries })
}

fn apply(skills: &[Planned], target: &Path, dry_run: bool, force: bool) -> Result<Summary> {
    let mut state = State::load(target)?;
    if !dry_run {
        std::fs::create_dir_all(target)
            .with_context(|| format!("cannot create {}", target.display()))?;
        deploy::sweep_staging(target)?;
    }

    let mut changes = Vec::new();
    for planned in skills {
        let action = install(target, &mut state, planned, dry_run, force)?;
        changes.push(Change {
            action,
            name: planned.name.clone(),
            origin: planned.origin(),
        });
    }

    let declared: BTreeSet<&str> = skills.iter().map(|skill| skill.name.as_str()).collect();
    let orphans: Vec<String> = state
        .skills
        .keys()
        .filter(|name| !declared.contains(name.as_str()))
        .cloned()
        .collect();
    for name in orphans {
        let destination = target.join(&name);
        if dry_run {
            tracing::info!(skill = %name, path = %destination.display(), "would remove");
        } else {
            tracing::info!(skill = %name, path = %destination.display(), "removing");
            deploy::remove(&destination)?;
        }

        state.skills.remove(&name);
        changes.push(Change {
            action: Action::Removed,
            name,
            origin: String::new(),
        });
    }

    if !dry_run {
        state.save(target)?;
    }

    Ok(Summary {
        target: target.to_path_buf(),
        changes,
        dry_run,
    })
}

fn install(
    target: &Path,
    state: &mut State,
    planned: &Planned,
    dry_run: bool,
    force: bool,
) -> Result<Action> {
    let _span = tracing::info_span!("target", path = %target.display()).entered();
    let destination = target.join(&planned.name);
    let known = state.skills.get(&planned.name);

    if known.is_none() && destination.exists() && !force {
        bail!(
            "{} already exists and skillmgr did not install it; move it aside, or pass --force to take it over",
            destination.display()
        );
    }

    let action = if known.is_some_and(|record| record.checksum == planned.checksum)
        && destination.is_dir()
        && deploy::checksum(&destination)? == planned.checksum
    {
        Action::Unchanged
    } else if destination.exists() {
        Action::Updated
    } else {
        Action::Added
    };

    if action == Action::Unchanged {
        tracing::debug!(skill = %planned.name, "unchanged");
    } else if dry_run {
        tracing::info!(skill = %planned.name, origin = %planned.origin(), action = action.label(), "would install");
    } else {
        tracing::info!(skill = %planned.name, origin = %planned.origin(), action = action.label(), "installing");
        deploy::install(&planned.source_dir, &destination)?;
    }

    state.skills.insert(
        planned.name.clone(),
        InstalledSkill {
            repo: planned.repo.clone(),
            revision: planned.revision.clone(),
            commit: planned.commit.clone(),
            source_path: planned.source_path.clone(),
            checksum: planned.checksum.clone(),
        },
    );
    Ok(action)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{PathSpec, RepoSpec};
    use crate::source::{Materialized, MockMaterializer};
    use crate::state::State;

    fn write_skill(root: &Path, name: &str, description: &str) {
        let dir = root.join("skills").join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {description}\n---\n"),
        )
        .unwrap();
    }

    fn config_with(repo_count: usize) -> Config {
        Config {
            targets: None,
            repos: (0..repo_count)
                .map(|index| RepoSpec {
                    repo: format!("https://example.invalid/{index}.git"),
                    revision: Some("1.0.0".to_owned()),
                    paths: vec![PathSpec {
                        path: PathBuf::from("skills"),
                        recurse: false,
                        exclude: None,
                    }],
                })
                .collect(),
            base_dir: PathBuf::new(),
        }
    }

    fn materializer(root: &Path) -> MockMaterializer {
        let root = root.to_path_buf();
        let mut mock = MockMaterializer::new();
        mock.expect_materialize().returning(move |_| {
            let materialized = Materialized {
                root: root.clone(),
                resolved: Some("c0ffee".to_owned()),
            };
            Box::pin(async move { Ok(materialized) })
        });
        mock
    }

    fn one(target: &Path) -> Vec<PathBuf> {
        vec![target.to_path_buf()]
    }

    fn actions(report: &Report) -> Vec<(Action, &str)> {
        report.summaries[0]
            .changes
            .iter()
            .map(|change| (change.action, change.name.as_str()))
            .collect()
    }

    #[tokio::test]
    async fn installs_a_skill_and_then_reports_it_unchanged() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        write_skill(source.path(), "demo", "A demo skill.");
        let config = config_with(1);
        let mock = materializer(source.path());

        let first = update(&config, &one(target.path()), &mock, false, false)
            .await
            .unwrap();
        let second = update(&config, &one(target.path()), &mock, false, false)
            .await
            .unwrap();

        assert_eq!(actions(&first), [(Action::Added, "demo")]);
        assert_eq!(actions(&second), [(Action::Unchanged, "demo")]);
        assert!(target.path().join("demo/SKILL.md").is_file());
        assert!(
            State::load(target.path())
                .unwrap()
                .skills
                .contains_key("demo")
        );
    }

    #[tokio::test]
    async fn reinstalls_when_the_source_moves_on() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        write_skill(source.path(), "demo", "A demo skill.");
        let config = config_with(1);
        let mock = materializer(source.path());

        update(&config, &one(target.path()), &mock, false, false)
            .await
            .unwrap();
        write_skill(source.path(), "demo", "A better demo skill.");
        let summary = update(&config, &one(target.path()), &mock, false, false)
            .await
            .unwrap();

        assert_eq!(actions(&summary), [(Action::Updated, "demo")]);
        assert!(
            std::fs::read_to_string(target.path().join("demo/SKILL.md"))
                .unwrap()
                .contains("A better demo skill.")
        );
    }

    #[tokio::test]
    async fn reinstalls_when_the_installed_copy_was_edited_by_hand() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        write_skill(source.path(), "demo", "A demo skill.");
        let config = config_with(1);
        let mock = materializer(source.path());

        update(&config, &one(target.path()), &mock, false, false)
            .await
            .unwrap();
        std::fs::write(target.path().join("demo/SKILL.md"), "tampered").unwrap();
        let summary = update(&config, &one(target.path()), &mock, false, false)
            .await
            .unwrap();

        assert_eq!(actions(&summary), [(Action::Updated, "demo")]);
    }

    #[tokio::test]
    async fn prunes_a_skill_the_config_no_longer_selects() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        write_skill(source.path(), "demo", "A demo skill.");
        write_skill(source.path(), "gone", "On its way out.");
        let config = config_with(1);
        let mock = materializer(source.path());

        update(&config, &one(target.path()), &mock, false, false)
            .await
            .unwrap();
        std::fs::remove_dir_all(source.path().join("skills/gone")).unwrap();
        let summary = update(&config, &one(target.path()), &mock, false, false)
            .await
            .unwrap();

        assert_eq!(
            actions(&summary),
            [(Action::Unchanged, "demo"), (Action::Removed, "gone")]
        );
        assert!(!target.path().join("gone").exists());
        assert!(target.path().join("demo").exists());
    }

    #[tokio::test]
    async fn refuses_to_take_over_a_directory_it_does_not_own() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        write_skill(source.path(), "demo", "A demo skill.");
        std::fs::create_dir_all(target.path().join("demo")).unwrap();
        std::fs::write(target.path().join("demo/SKILL.md"), "handwritten").unwrap();
        let config = config_with(1);
        let mock = materializer(source.path());

        let error = update(&config, &one(target.path()), &mock, false, false)
            .await
            .unwrap_err()
            .to_string();

        assert!(error.contains("--force"), "{error}");
        assert_eq!(
            std::fs::read_to_string(target.path().join("demo/SKILL.md")).unwrap(),
            "handwritten"
        );
    }

    #[tokio::test]
    async fn takes_over_an_unmanaged_directory_when_forced() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        write_skill(source.path(), "demo", "A demo skill.");
        std::fs::create_dir_all(target.path().join("demo")).unwrap();
        std::fs::write(target.path().join("demo/SKILL.md"), "handwritten").unwrap();
        let config = config_with(1);
        let mock = materializer(source.path());

        let summary = update(&config, &one(target.path()), &mock, false, true)
            .await
            .unwrap();

        assert_eq!(actions(&summary), [(Action::Updated, "demo")]);
        assert!(
            std::fs::read_to_string(target.path().join("demo/SKILL.md"))
                .unwrap()
                .contains("A demo skill.")
        );
    }

    #[tokio::test]
    async fn a_dry_run_reports_without_writing() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        write_skill(source.path(), "demo", "A demo skill.");
        let config = config_with(1);
        let mock = materializer(source.path());

        let summary = update(&config, &one(target.path()), &mock, true, false)
            .await
            .unwrap();

        assert_eq!(actions(&summary), [(Action::Added, "demo")]);
        assert!(summary.summaries[0].dry_run);
        assert!(!target.path().join("demo").exists());
        assert!(!State::path(target.path()).exists());
    }

    #[tokio::test]
    async fn refuses_two_sources_providing_the_same_skill() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        write_skill(source.path(), "demo", "A demo skill.");
        let config = config_with(2);
        let mock = materializer(source.path());

        let error = update(&config, &one(target.path()), &mock, false, false)
            .await
            .unwrap_err()
            .to_string();

        assert!(
            error.contains("two sources provide the skill `demo`"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn refuses_a_skill_that_breaks_the_specification() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let dir = source.path().join("skills/Bad-Name");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            "---\nname: Bad-Name\ndescription: x\n---\n",
        )
        .unwrap();
        let config = config_with(1);
        let mock = materializer(source.path());

        let error = update(&config, &one(target.path()), &mock, false, false)
            .await
            .unwrap_err()
            .to_string();

        assert!(error.contains("Agent Skills specification"), "{error}");
        assert!(!target.path().join("Bad-Name").exists());
    }

    #[tokio::test]
    async fn deploys_the_same_skill_into_every_target() {
        let source = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        write_skill(source.path(), "demo", "A demo skill.");
        let claude = root.path().join(".claude/skills");
        let agents = root.path().join(".agents/skills");
        let config = config_with(1);
        let mock = materializer(source.path());

        let report = update(
            &config,
            &[claude.clone(), agents.clone()],
            &mock,
            false,
            false,
        )
        .await
        .unwrap();

        assert_eq!(report.summaries.len(), 2);
        assert_eq!(report.count(Action::Added), 2);
        assert!(claude.join("demo/SKILL.md").is_file());
        assert!(agents.join("demo/SKILL.md").is_file());
        assert!(State::load(&claude).unwrap().skills.contains_key("demo"));
        assert!(State::load(&agents).unwrap().skills.contains_key("demo"));
    }

    #[tokio::test]
    async fn a_second_target_added_later_catches_up_on_its_own() {
        let source = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        write_skill(source.path(), "demo", "A demo skill.");
        let claude = root.path().join(".claude/skills");
        let agents = root.path().join(".agents/skills");
        let config = config_with(1);
        let mock = materializer(source.path());

        update(&config, std::slice::from_ref(&claude), &mock, false, false)
            .await
            .unwrap();
        let report = update(&config, &[claude, agents], &mock, false, false)
            .await
            .unwrap();

        assert_eq!(report.count(Action::Unchanged), 1);
        assert_eq!(report.count(Action::Added), 1);
    }

    #[test]
    fn the_summary_counts_every_action() {
        let summary = Summary {
            target: PathBuf::from(".claude/skills"),
            changes: vec![
                Change {
                    action: Action::Added,
                    name: "alpha".to_owned(),
                    origin: "local skills/alpha".to_owned(),
                },
                Change {
                    action: Action::Removed,
                    name: "beta".to_owned(),
                    origin: String::new(),
                },
            ],
            dry_run: false,
        };

        let rendered = summary.to_string();

        assert!(rendered.contains("+ alpha"), "{rendered}");
        assert!(rendered.contains("- beta"), "{rendered}");
        assert!(
            rendered.contains("1 skill in .claude/skills: 1 added, 1 removed"),
            "{rendered}"
        );
    }
}
