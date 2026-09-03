//! The command line surface.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// Deploy Agent Skills declared in `skillmgr.yaml`.
#[derive(Debug, Parser)]
#[command(name = "skillmgr", version, about, long_about = None)]
pub struct Cli {
    /// Path to the configuration file.
    #[arg(
        long,
        short = 'c',
        env = "SKILLMGR_CONFIG_FILE",
        default_value = "skillmgr.yaml",
        global = true
    )]
    pub config: PathBuf,

    /// Directory to deploy into, overriding the config's `targets`.
    ///
    /// Repeat the flag for several directories; the environment variable
    /// takes a colon-separated list.
    #[arg(
        long = "target",
        env = "SKILLMGR_SKILLS_DIR",
        value_delimiter = ':',
        global = true
    )]
    pub targets: Vec<PathBuf>,

    /// Directory holding the git checkouts skillmgr reuses between runs.
    #[arg(long, env = "SKILLMGR_CACHE_DIR", global = true)]
    pub cache_dir: Option<PathBuf>,

    /// Work from the cache only, without contacting any git remote.
    #[arg(long, env = "SKILLMGR_OFFLINE", global = true)]
    pub offline: bool,

    /// `tracing` filter directive (e.g. `info`, `skillmgr=debug`).
    ///
    /// Syntax: <https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html#directives>
    #[arg(
        long = "log-filter",
        env = "SKILLMGR_LOG_FILTER",
        default_value = "info",
        global = true
    )]
    pub log_filter: String,

    /// What to do.
    #[command(subcommand)]
    pub command: Command,
}

/// The subcommands skillmgr exposes.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Install, refresh and prune the declared skills.
    Update {
        /// Report what would change without touching the target directory.
        #[arg(long, env = "SKILLMGR_DRY_RUN")]
        dry_run: bool,

        /// Take ownership of a skill directory skillmgr did not install.
        #[arg(long, env = "SKILLMGR_FORCE")]
        force: bool,
    },

    /// Show the skills skillmgr manages in the target directory.
    List,

    /// Check the config and every skill it selects, without deploying.
    Validate {
        /// Config files to check, instead of the one `--config` names.
        #[arg(value_name = "CONFIG")]
        configs: Vec<PathBuf>,

        /// Check the config files alone, without fetching any source.
        ///
        /// This is the fast, network-free check a pre-commit hook wants: it
        /// proves the file parses and its rules hold, and says nothing about
        /// the skills the sources would yield.
        #[arg(long, env = "SKILLMGR_CONFIG_ONLY")]
        config_only: bool,
    },

    /// Print the JSON Schema for `skillmgr.yaml`.
    Schema,
}
