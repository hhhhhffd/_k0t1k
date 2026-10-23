//! Core of the _k0t1k launcher: everything that does not depend on the UI shell.

pub mod component;
pub mod download;
pub mod error;
pub mod instance;
pub mod java;
pub mod paths;
pub mod progress;
pub mod settings;

pub use error::{Error, Result};
