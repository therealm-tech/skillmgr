//! Deploy Agent Skills from git repositories and local directories,
//! declaratively, from a single `skillmgr.yaml`.

pub mod cli;
pub mod command;
pub mod config;
pub mod deploy;
pub mod discovery;
pub mod schema;
pub mod shutdown;
pub mod skill;
pub mod source;
pub mod state;
