//! The subcommand implementations.

pub mod list;
pub mod plan;
pub mod schema;
pub mod update;
pub mod validate;

use std::path::PathBuf;

use crate::cli::Cli;
use crate::config::{Config, expand_home};
use crate::source::{Fetcher, default_cache_dir};

/// Everything a subcommand needs, resolved from the CLI and the config file.
pub struct Context {
    /// The parsed configuration.
    pub config: Config,
    /// The directories skills are deployed into.
    pub targets: Vec<PathBuf>,
}

impl Context {
    /// Load the config and resolve the target directory.
    ///
    /// # Errors
    ///
    /// When the config file cannot be read, parsed or validated.
    pub fn open(cli: &Cli) -> anyhow::Result<Self> {
        Self::open_at(cli, &cli.config)
    }

    /// Load a named config file and resolve the target directories.
    ///
    /// # Errors
    ///
    /// When the config file cannot be read, parsed or validated.
    pub fn open_at(cli: &Cli, config_path: &std::path::Path) -> anyhow::Result<Self> {
        let config = Config::load(config_path)?;
        let targets = config.targets(&cli.targets);
        tracing::debug!(
            targets = ?targets,
            config = %config_path.display(),
            "resolved"
        );
        Ok(Self { config, targets })
    }

    /// The materializer the config's sources are fetched through.
    #[must_use]
    pub fn fetcher(&self, cli: &Cli) -> Fetcher {
        let cache = cli
            .cache_dir
            .as_deref()
            .map_or_else(default_cache_dir, expand_home);
        Fetcher::new(self.config.base_dir.clone(), cache, cli.offline)
    }
}
