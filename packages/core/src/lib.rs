//! Core of the _k0t1k launcher: everything that does not depend on the UI shell.

pub mod auth;
pub mod component;
pub mod content;
pub mod download;
pub mod error;
pub mod install;
pub mod instance;
pub mod java;
pub mod launch;
pub mod meta;
pub mod paths;
pub mod placement;
pub mod profile;
pub mod progress;
pub mod settings;

pub use error::{Error, Result};
