//! `skillmgr list`: what is deployed in the target directories.

use anyhow::Result;

use crate::cli::Cli;
use crate::command::Context;
use crate::state::{InstalledSkill, State};

/// Run the subcommand.
///
/// # Errors
///
/// When the config or a target directory's state cannot be read.
pub fn run(cli: &Cli) -> Result<()> {
    let context = Context::open(cli)?;

    for (index, target) in context.targets.iter().enumerate() {
        let state = State::load(target)?;
        tracing::debug!(target = %target.display(), count = state.skills.len(), "installed skills");

        if index > 0 {
            println!();
        }
        println!("{}:", target.display());

        let width = state.skills.keys().map(String::len).max().unwrap_or(0);
        for (name, skill) in &state.skills {
            println!("  {name:width$}  {}  {}", origin(skill), skill.source_path);
        }
    }
    Ok(())
}

fn origin(skill: &InstalledSkill) -> String {
    match (&skill.revision, &skill.commit) {
        (Some(revision), Some(commit)) => format!(
            "{}@{revision} ({})",
            skill.repo,
            &commit[..commit.len().min(8)]
        ),
        (Some(revision), None) => format!("{}@{revision}", skill.repo),
        _ => skill.repo.clone(),
    }
}
